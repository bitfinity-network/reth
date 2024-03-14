//! Common CLI utility functions.

use alloy_chains::Chain;
use boyer_moore_magiclen::BMByte;
use ethereum_json_rpc_client::reqwest::ReqwestClient;
use eyre::Result;
use reth_db::{
    cursor::{DbCursorRO, DbDupCursorRO},
    database::Database,
    table::{Decode, Decompress, DupSort, Table, TableRow},
    transaction::{DbTx, DbTxMut},
    DatabaseError, RawTable, TableRawRow,
};
use reth_primitives::ruint::Uint;
use reth_primitives::{
    fs, ChainConfig, ChainSpec, ForkCondition,  Genesis, GenesisAccount, Hardfork,
    U256,
};
use serde_json::json;
use std::collections::BTreeMap;

use std::{path::Path, rc::Rc, sync::Arc};
use tracing::info;

/// Exposing `open_db_read_only` function
pub mod db {
    pub use reth_db::open_db_read_only;
}

/// Re-exported from `reth_node_core`, also to prevent a breaking change. See the comment on
/// the `reth_node_core::args` re-export for more details.
pub use reth_node_core::utils::*;

/// Wrapper over DB that implements many useful DB queries.
#[derive(Debug)]
pub struct DbTool<'a, DB: Database> {
    /// The database that the db tool will use.
    pub db: &'a DB,
    /// The [ChainSpec] that the db tool will use.
    pub chain: Arc<ChainSpec>,
}

impl<'a, DB: Database> DbTool<'a, DB> {
    /// Takes a DB where the tables have already been created.
    pub fn new(db: &'a DB, chain: Arc<ChainSpec>) -> eyre::Result<Self> {
        Ok(Self { db, chain })
    }

    /// Grabs the contents of the table within a certain index range and places the
    /// entries into a [`HashMap`][std::collections::HashMap].
    ///
    /// [`ListFilter`] can be used to further
    /// filter down the desired results. (eg. List only rows which include `0xd3adbeef`)
    pub fn list<T: Table>(&self, filter: &ListFilter) -> Result<(Vec<TableRow<T>>, usize)> {
        let bmb = Rc::new(BMByte::from(&filter.search));
        if bmb.is_none() && filter.has_search() {
            eyre::bail!("Invalid search.")
        }

        let mut hits = 0;

        let data = self.db.view(|tx| {
            let mut cursor =
                tx.cursor_read::<RawTable<T>>().expect("Was not able to obtain a cursor.");

            let map_filter = |row: Result<TableRawRow<T>, _>| {
                if let Ok((k, v)) = row {
                    let (key, value) = (k.into_key(), v.into_value());

                    if key.len() + value.len() < filter.min_row_size {
                        return None
                    }
                    if key.len() < filter.min_key_size {
                        return None
                    }
                    if value.len() < filter.min_value_size {
                        return None
                    }

                    let result = || {
                        if filter.only_count {
                            return None
                        }
                        Some((
                            <T as Table>::Key::decode(&key).unwrap(),
                            <T as Table>::Value::decompress(&value).unwrap(),
                        ))
                    };

                    match &*bmb {
                        Some(searcher) => {
                            if searcher.find_first_in(&value).is_some() ||
                                searcher.find_first_in(&key).is_some()
                            {
                                hits += 1;
                                return result()
                            }
                        }
                        None => {
                            hits += 1;
                            return result()
                        }
                    }
                }
                None
            };

            if filter.reverse {
                Ok(cursor
                    .walk_back(None)?
                    .skip(filter.skip)
                    .filter_map(map_filter)
                    .take(filter.len)
                    .collect::<Vec<(_, _)>>())
            } else {
                Ok(cursor
                    .walk(None)?
                    .skip(filter.skip)
                    .filter_map(map_filter)
                    .take(filter.len)
                    .collect::<Vec<(_, _)>>())
            }
        })?;

        Ok((data.map_err(|e: DatabaseError| eyre::eyre!(e))?, hits))
    }

    /// Grabs the content of the table for the given key
    pub fn get<T: Table>(&self, key: T::Key) -> Result<Option<T::Value>> {
        self.db.view(|tx| tx.get::<T>(key))?.map_err(|e| eyre::eyre!(e))
    }

    /// Grabs the content of the DupSort table for the given key and subkey
    pub fn get_dup<T: DupSort>(&self, key: T::Key, subkey: T::SubKey) -> Result<Option<T::Value>> {
        self.db
            .view(|tx| tx.cursor_dup_read::<T>()?.seek_by_key_subkey(key, subkey))?
            .map_err(|e| eyre::eyre!(e))
    }

    /// Drops the database at the given path.
    pub fn drop(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        info!(target: "reth::cli", "Dropping database at {:?}", path);
        fs::remove_dir_all(path)?;
        Ok(())
    }

    /// Drops the provided table from the database.
    pub fn drop_table<T: Table>(&mut self) -> Result<()> {
        self.db.update(|tx| tx.clear::<T>())??;
        Ok(())
    }
}

/// Filters the results coming from the database.
#[derive(Debug)]
pub struct ListFilter {
    /// Skip first N entries.
    pub skip: usize,
    /// Take N entries.
    pub len: usize,
    /// Sequence of bytes that will be searched on values and keys from the database.
    pub search: Vec<u8>,
    /// Minimum row size.
    pub min_row_size: usize,
    /// Minimum key size.
    pub min_key_size: usize,
    /// Minimum value size.
    pub min_value_size: usize,
    /// Reverse order of entries.
    pub reverse: bool,
    /// Only counts the number of filtered entries without decoding and returning them.
    pub only_count: bool,
}

impl ListFilter {
    /// If `search` has a list of bytes, then filter for rows that have this sequence.
    pub fn has_search(&self) -> bool {
        !self.search.is_empty()
    }

    /// Updates the page with new `skip` and `len` values.
    pub fn update_page(&mut self, skip: usize, len: usize) {
        self.skip = skip;
        self.len = len;
    }
}

/// Bitfinity Genesis
pub async fn bitfinity_genesis(url: String) -> Result<Arc<ChainSpec>> {
    let client = ethereum_json_rpc_client::EthJsonRpcClient::new(ReqwestClient::new(url));

    let chain_id = client.get_chain_id().await.map_err(|e| eyre::eyre!(e))?;

    tracing::info!("Bitfinity chain id: {}", chain_id);

    let genesis_block = client
        .get_block_by_number(0.into())
        .await
        .map_err(|e| eyre::eyre!("error getting genesis block: {}", e))?;

    let genesis_accounts = client
        .get_genesis_balances()
        .await
        .map_err(|e| eyre::eyre!(e))?
        .into_iter()
        .map(|(k, v)| {
            tracing::info!("Bitfinity genesis account: {:?} {:?}", k, v);
            (k.0.into(), GenesisAccount { balance: Uint::from_limbs(v.0), ..Default::default() })
        });

    let chain = Chain::from_id(chain_id);

    let mut genesis: Genesis = serde_json::from_value(json!(genesis_block))
        .map_err(|e| eyre::eyre!("error parsing genesis block: {}", e))?;

    tracing::info!("Bitfinity genesis: {:?}", genesis);

    genesis.config = ChainConfig {
        chain_id,
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        muir_glacier_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        arrow_glacier_block: Some(0),
        gray_glacier_block: Some(0),
        merge_netsplit_block: Some(0),
        terminal_total_difficulty: Some(Uint::ZERO),
        terminal_total_difficulty_passed: true,
        ..Default::default()
    };

    genesis.alloc = genesis_accounts.collect();

    let spec = ChainSpec {
        chain,
        genesis_hash: genesis_block.hash.map(|h| h.0.into()),
        genesis: genesis.clone(),
        paris_block_and_final_difficulty: Some((0, Uint::ZERO)),
        hardforks: BTreeMap::from([
            (Hardfork::Frontier, ForkCondition::Block(0)),
            (Hardfork::Homestead, ForkCondition::Block(0)),
            (Hardfork::Dao, ForkCondition::Block(0)),
            (Hardfork::Tangerine, ForkCondition::Block(0)),
            (Hardfork::SpuriousDragon, ForkCondition::Block(0)),
            (Hardfork::Byzantium, ForkCondition::Block(0)),
            (Hardfork::Constantinople, ForkCondition::Block(0)),
            (Hardfork::Petersburg, ForkCondition::Block(0)),
            (Hardfork::Istanbul, ForkCondition::Block(0)),
            (Hardfork::Berlin, ForkCondition::Block(0)),
            (Hardfork::London, ForkCondition::Block(0)),
            (
                Hardfork::Paris,
                ForkCondition::TTD { fork_block: Some(0), total_difficulty: U256::from(0) },
            ),
        ]),
        ..Default::default()
    };

    tracing::debug!("Bitfinity genesis: {:?}", spec);

    Ok(Arc::new(spec))
}
