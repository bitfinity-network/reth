use candid::Principal;
use did::certified::CertifiedResult;
use ethereum_json_rpc_client::{reqwest::ReqwestClient, EthJsonRpcClient};
use eyre::Result;
use ic_cbor::{CertificateToCbor, HashTreeToCbor};
use ic_certificate_verification::VerifyCertificate;
use ic_certification::{Certificate, HashTree, LookupResult};
use itertools::Either;
use rayon::iter::{IntoParallelIterator, ParallelIterator as _};
use reth_chainspec::{
    BaseFeeParams, BaseFeeParamsKind, Chain, ChainHardforks, ChainSpec, EthereumHardfork,
};

use alloy_rlp::Decodable;
use parking_lot::{Mutex, RwLock};
use reth_network_p2p::{
    bodies::client::{BodiesClient, BodiesFut},
    download::DownloadClient,
    error::RequestError,
    headers::client::{HeadersClient, HeadersFut, HeadersRequest},
    priority::Priority,
};
use reth_network_peers::PeerId;
use reth_primitives::{
    ruint::Uint, BlockBody, BlockHash, BlockHashOrNumber, BlockNumber, ChainConfig, ForkCondition,
    Genesis, GenesisAccount, Header, HeadersDirection, B256, U256,
};
use rlp::Encodable;
use serde_json::json;
use std::fmt::Debug;
use std::future::Future;

use std::{self, cmp::min, collections::HashMap, time::Duration};
use thiserror::Error;
use tracing::{debug, error, info, trace, warn};
/// Front-end API for fetching chain data from remote sources.
///
/// Blocks are assumed to have populated transactions, so reading headers will also buffer
/// transactions in memory for use in the bodies stage.
pub struct BitfinityEvmClient {
    /// The RPC client configuration
    rpc_config: RpcClientConfig,
    /// The current active client
    client: RwLock<EthJsonRpcClient<ReqwestClient>>,
    /// Currently using backup URL
    using_backup: Mutex<bool>,
    /// The buffered headers retrieved when fetching new bodies.
    headers: HashMap<BlockNumber, Header>,
    /// A mapping between block hash and number.
    hash_to_number: HashMap<BlockHash, BlockNumber>,
    /// The buffered bodies retrieved when fetching new headers.
    bodies: HashMap<BlockHash, BlockBody>,
}

impl Debug for BitfinityEvmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BitfinityEvmClient")
            .field("rpc_config", &self.rpc_config)
            .field("headers", &self.headers)
            .field("hash_to_number", &self.hash_to_number)
            .field("bodies", &self.bodies)
            .finish()
    }
}

/// An error that can occur when constructing and using a [`RemoteClient`].
#[derive(Debug, Error)]
pub enum RemoteClientError {
    /// An error occurred when decoding blocks, headers, or rlp headers.
    #[error(transparent)]
    Rlp(#[from] alloy_rlp::Error),

    /// An error occurred when fetching blocks, headers, or rlp headers from the remote provider.
    #[error("provider error occurred: {0}")]
    ProviderError(String),

    /// Certificate check error
    #[error("certification check error: {0}")]
    CertificateError(String),

    /// Internal error
    #[error("internal error: {0}")]
    InternalError(String),
}

/// Setting for checking last certified block
#[derive(Debug)]
pub struct CertificateCheckSettings {
    /// Principal of the EVM canister
    pub evmc_principal: String,
    /// Root key of the IC network
    pub ic_root_key: String,
}

/// RPC client configuration
#[derive(Debug, Clone)]
pub struct RpcClientConfig {
    /// Primary RPC URL
    pub primary_url: String,
    /// Backup RPC URL
    pub backup_url: Option<String>,
    /// Maximum number of retries
    pub max_retries: u32,
    /// Delay between retries
    pub retry_delay: Duration,
}

impl BitfinityEvmClient {
    /// Creates a new RPC client for the given URL
    fn create_client(url: &str) -> EthJsonRpcClient<ReqwestClient> {
        EthJsonRpcClient::new(ReqwestClient::new(url.to_string()))
    }

    /// Create a new `BitfinityEvmClient` from RPC configuration
    pub fn new(config: RpcClientConfig) -> Self {
        let client = Self::create_client(&config.primary_url);

        Self {
            rpc_config: config,
            client: RwLock::new(client),
            using_backup: Mutex::new(false),
            headers: HashMap::new(),
            hash_to_number: HashMap::new(),
            bodies: HashMap::new(),
        }
    }

    /// Try to switch back to primary URL if we're using backup
    async fn try_primary_url(&self) -> Result<(), RemoteClientError> {
        // Early return if not using backup
        {
            let backup_status = self.using_backup.lock();
            if !*backup_status {
                return Ok(());
            }
        }
        debug!(target: "downloaders::bitfinity_evm_client", "Attempting to switch back to primary URL");

        // Create new client and test connection
        let new_client = Self::create_client(&self.rpc_config.primary_url);
        match new_client.get_block_number().await {
            Ok(_) => {
                // Update client and status atomically
                let mut client = self.client.try_write().ok_or_else(|| {
                    RemoteClientError::InternalError("Failed to acquire `client` lock".to_string())
                })?;

                let mut using_backup = self.using_backup.try_lock().ok_or_else(|| {
                    RemoteClientError::InternalError(
                        "Failed to acquire `using_backup` lock".to_string(),
                    )
                })?;

                *client = new_client;
                *using_backup = false;
                info!(target: "downloaders::bitfinity_evm_client", "Successfully switched back to primary URL");
                Ok(())
            }
            Err(e) => {
                warn!(target: "downloaders::bitfinity_evm_client", "Failed to switch back to primary URL: {}, Primary URL still unavailable", e);

                Ok(())
            }
        }
    }

    /// Switch to backup URL if available/// Switch to backup URL if available
    async fn switch_to_backup(&self) -> Result<(), RemoteClientError> {
        let backup_url = self.rpc_config.backup_url.as_ref().ok_or_else(|| {
            RemoteClientError::ProviderError("No backup URL configured".to_string())
        })?;

        // Early return if already using backup
        {
            let using_backup = self.using_backup.lock();

            if *using_backup {
                return Err(RemoteClientError::ProviderError(
                    "Already using backup URL".to_string(),
                ));
            }
        }
        warn!(target: "downloaders::bitfinity_evm_client", "Switching to backup RPC URL");

        // Create and test backup client
        let backup_client = Self::create_client(backup_url);
        match backup_client.get_block_number().await {
            Ok(_) => {
                // Update client and status atomically
                let mut client = self.client.write();

                let mut using_backup = self.using_backup.lock();

                *client = backup_client;
                *using_backup = true;
                info!(target: "downloaders::bitfinity_evm_client", "Successfully switched to backup URL");
                Ok(())
            }
            Err(e) => {
                error!(target: "downloaders::bitfinity_evm_client", "Failed to switch to backup URL: {}", e);
                Err(RemoteClientError::ProviderError(format!("Backup URL check failed: {}", e)))
            }
        }
    }

    /// Execute an RPC call with retries and backup URL support
    async fn execute_with_retry<F, R, T, E>(&self, operation: F) -> Result<T, RemoteClientError>
    where
        F: Fn(&EthJsonRpcClient<ReqwestClient>) -> R + Send + Sync,
        R: Future<Output = Result<T, E>> + Send,
        T: Send,
        E: std::fmt::Display, // This workaround is necessary because anyhow::Error does not implement the `std::error::Error`trait
    {
        let mut retries = 0;
        let max_retries = self.rpc_config.max_retries;
        let delay = self.rpc_config.retry_delay;

        loop {
            let result = {
                let client = self.client.try_read().ok_or_else(|| {
                    RemoteClientError::InternalError("Failed to acquire `client` lock".to_string())
                })?;

                operation(&client)
            };

            match result.await {
                Ok(result) => return Ok(result),
                Err(e) => {
                    if retries >= max_retries {
                        // Try backup if primary failed
                        if !*self.using_backup.lock() {
                            match self.switch_to_backup().await {
                                Ok(()) => {
                                    retries = 0;
                                    continue;
                                }
                                Err(backup_err) => {
                                    return Err(RemoteClientError::ProviderError(format!(
                                        "Primary failed after {} retries: {}. Backup failed: {}",
                                        max_retries, e, backup_err
                                    )));
                                }
                            }
                        }
                        return Err(RemoteClientError::ProviderError(format!(
                            "Operation failed after {} retries: {}",
                            retries, e
                        )));
                    }

                    warn!(
                        target: "downloaders::bitfinity_evm_client",
                        "RPC call failed (attempt {}/{}), retrying in {:?}: {}",
                        retries + 1, max_retries, delay, e
                    );

                    tokio::time::sleep(delay).await;
                    retries += 1;
                }
            }
        }
    }

    /// `BitfinityEvmClient` from rpc url
    pub async fn from_rpc_url(
        config: RpcClientConfig,
        start_block: u64,
        end_block: Option<u64>,
        batch_size: usize,
        max_blocks: u64,
        certificate_settings: Option<CertificateCheckSettings>,
    ) -> Result<Self, RemoteClientError> {
        let mut headers = HashMap::new();
        let mut hash_to_number = HashMap::new();
        let mut bodies = HashMap::new();

        let client = Self::new(config.clone());

        let block_checker = match certificate_settings {
            None => None,
            Some(settings) => Some(BlockCertificateChecker::new(&client, settings).await?),
        };

        let latest_remote_block = client
            .execute_with_retry(|client| {
                let client = client.clone();
                async move { client.get_block_number().await }
            })
            .await?;

        let mut end_block =
            min(end_block.unwrap_or(latest_remote_block), start_block + max_blocks - 1);

        if end_block < start_block {
            return Ok(Self {
                rpc_config: config,
                client: client.client,
                using_backup: client.using_backup,
                headers,
                hash_to_number,
                bodies,
            });
        }

        if let Some(block_checker) = &block_checker {
            end_block = min(end_block, block_checker.get_block_number());
        }

        info!(target: "downloaders::bitfinity_evm_client", "Start fetching blocks from {} to {}", start_block, end_block);

        for begin_block in (start_block..=end_block).step_by(batch_size) {
            let count = std::cmp::min(batch_size as u64, end_block + 1 - begin_block);
            let last_block = begin_block + count - 1;

            // Try switching back to primary URL if we're using backup
            client.try_primary_url().await?;

            debug!(target: "downloaders::bitfinity_evm_client", "Fetching blocks from {} to {}", begin_block, last_block);

            let full_blocks = client
                .execute_with_retry(|client|{
                    let client = client.clone();
                    let blocks_to_fetch = (begin_block..=last_block).map(Into::into);

                    async move { client.get_full_blocks_by_number(blocks_to_fetch, batch_size)
                    .await
                }})
                .await
                .map_err(|e| {
                    error!(target: "downloaders::bitfinity_evm_client", begin_block, "Error fetching block: {:?}", e);
                    RemoteClientError::ProviderError(format!(
                        "Error fetching block {}: {:?}",
                        begin_block, e
                    ))
                })?
                .into_par_iter()
                .clone()
                .map(did::Block::<did::Transaction>::from)
                .collect::<Vec<_>>();

            trace!(target: "downloaders::bitfinity_evm_client", blocks = full_blocks.len(), "Fetched blocks");

            for block in full_blocks {
                if let Some(block_checker) = &block_checker {
                    block_checker.check_block(&block)?;
                }

                let header =
                    reth_primitives::Block::decode(&mut block.rlp_bytes().to_vec().as_slice())?;

                let block_hash = header.hash_slow();

                headers.insert(header.number, header.header.clone());
                hash_to_number.insert(block_hash, header.number);
                bodies.insert(
                    block_hash,
                    BlockBody {
                        transactions: header.body,
                        ommers: header.ommers,
                        withdrawals: header.withdrawals,
                        requests: header.requests,
                    },
                );
            }
        }

        info!(blocks = headers.len(), "Initialized remote client");

        Ok(Self {
            rpc_config: config,
            client: client.client,
            using_backup: client.using_backup,
            headers,
            hash_to_number,
            bodies,
        })
    }

    /// Get the remote tip hash of the chain.
    pub fn tip(&self) -> Option<B256> {
        self.headers
            .keys()
            .max()
            .and_then(|max_key| self.headers.get(max_key))
            .map(|h| h.hash_slow())
    }

    /// Returns the highest block number of this client has or `None` if empty
    pub fn max_block(&self) -> Option<u64> {
        self.headers.keys().max().copied()
    }

    /// Returns true if all blocks are canonical (no gaps)
    pub fn has_canonical_blocks(&self) -> bool {
        if self.headers.is_empty() {
            return true;
        }
        let mut nums = self.headers.keys().copied().collect::<Vec<_>>();
        nums.sort_unstable();
        let mut iter = nums.into_iter();
        let mut lowest = iter.next().expect("not empty");
        for next in iter {
            if next != lowest + 1 {
                return false;
            }
            lowest = next;
        }
        true
    }

    /// Fetch Bitfinity chain spec
    pub async fn fetch_chain_spec(rpc: String) -> Result<ChainSpec> {
        Self::build_chain_spec(&rpc).await
    }

    /// Fetch Bitfinity chain spec with fallback
    pub async fn fetch_chain_spec_with_fallback(
        primary_rpc: String,
        backup_rpc: Option<String>,
    ) -> Result<ChainSpec> {
        match Self::build_chain_spec(&primary_rpc).await {
            Ok(spec) => Ok(spec),
            Err(e) => {
                warn!(target: "downloaders::bitfinity_evm_client", "Failed to fetch chain spec from primary URL: {}. Trying backup URL", e);

                if let Some(backup_rpc) = backup_rpc {
                    match Self::build_chain_spec(&backup_rpc).await {
                        Ok(spec) => Ok(spec),
                        Err(e) => {
                            error!(target: "downloaders::bitfinity_evm_client", "Failed to fetch chain spec from backup URL: {}", e);

                            Err(e)
                        }
                    }
                } else {
                    error!(target: "downloaders::bitfinity_evm_client", "No backup URL provided, failed to fetch chain spec from primary URL: {}", e);

                    Err(e)
                }
            }
        }
    }

    async fn build_chain_spec(url: &str) -> Result<ChainSpec> {
        let rpc_client = Self::create_client(url);

        let chain_id = rpc_client
            .get_chain_id()
            .await
            .map_err(|e| eyre::eyre!("error getting chain id: {}", e))?;

        tracing::info!("downloaders::bitfinity_evm_client - Bitfinity chain id: {}", chain_id);

        let genesis_block = rpc_client
            .get_block_by_number(0.into())
            .await
            .map_err(|e| eyre::eyre!("error getting genesis block: {}", e))?;

        let genesis_accounts = rpc_client
            .get_genesis_balances()
            .await
            .map_err(|e| eyre::eyre!("error getting genesis accounts: {}", e))?
            .into_iter()
            .map(|(k, v)| {
                tracing::info!(
                    "downloaders::bitfinity_evm_client - Bitfinity genesis account: {:?} {:?}",
                    k,
                    v
                );
                (
                    k.0.into(),
                    GenesisAccount { balance: Uint::from_limbs(v.0), ..Default::default() },
                )
            });

        let chain = Chain::from_id(chain_id);
        let mut genesis: Genesis = serde_json::from_value(json!(genesis_block))
            .map_err(|e| eyre::eyre!("error parsing genesis block: {}", e))?;

        genesis.config = ChainConfig {
            chain_id,
            homestead_block: Some(0),
            eip150_block: Some(0),
            eip155_block: Some(0),
            eip158_block: Some(0),
            terminal_total_difficulty: Some(Uint::ZERO),
            ..Default::default()
        };

        genesis.alloc = genesis_accounts.collect();

        Ok(ChainSpec {
            chain,
            genesis_hash: genesis_block.hash.map(|h| h.0.into()),
            genesis,
            paris_block_and_final_difficulty: Some((0, Uint::ZERO)),
            hardforks: ChainHardforks::new(vec![
                (EthereumHardfork::Frontier.boxed(), ForkCondition::Block(0)),
                (EthereumHardfork::Homestead.boxed(), ForkCondition::Block(0)),
                (EthereumHardfork::Dao.boxed(), ForkCondition::Block(0)),
                (EthereumHardfork::Tangerine.boxed(), ForkCondition::Block(0)),
                (EthereumHardfork::SpuriousDragon.boxed(), ForkCondition::Block(0)),
                (EthereumHardfork::Byzantium.boxed(), ForkCondition::Block(0)),
                (EthereumHardfork::Constantinople.boxed(), ForkCondition::Block(0)),
                (EthereumHardfork::Petersburg.boxed(), ForkCondition::Block(0)),
                (EthereumHardfork::Istanbul.boxed(), ForkCondition::Block(0)),
                (EthereumHardfork::Berlin.boxed(), ForkCondition::Block(0)),
                (EthereumHardfork::London.boxed(), ForkCondition::Block(0)),
                (
                    EthereumHardfork::Paris.boxed(),
                    ForkCondition::TTD { fork_block: Some(0), total_difficulty: U256::from(0) },
                ),
            ]),
            deposit_contract: None,
            base_fee_params: BaseFeeParamsKind::Constant(BaseFeeParams::ethereum()),
            prune_delete_limit: 0,
            bitfinity_evm_url: Some(url.to_string()),
        })
    }

    /// Use the provided bodies as the remote client's block body buffer.
    pub fn with_bodies(mut self, bodies: HashMap<BlockHash, BlockBody>) -> Self {
        self.bodies = bodies;
        self
    }

    /// Use the provided headers as the remote client's block body buffer.
    pub fn with_headers(mut self, headers: HashMap<BlockNumber, Header>) -> Self {
        self.headers = headers;
        for (number, header) in &self.headers {
            self.hash_to_number.insert(header.hash_slow(), *number);
        }
        self
    }
}

struct BlockCertificateChecker {
    certified_data: CertifiedResult<did::Block<did::H256>>,
    evmc_principal: Principal,
    ic_root_key: Vec<u8>,
}

impl BlockCertificateChecker {
    async fn new(
        client: &BitfinityEvmClient,
        certificate_settings: CertificateCheckSettings,
    ) -> Result<Self, RemoteClientError> {
        let evmc_principal =
            Principal::from_text(certificate_settings.evmc_principal).map_err(|e| {
                RemoteClientError::CertificateError(format!("failed to parse principal: {e}"))
            })?;
        let ic_root_key = hex::decode(&certificate_settings.ic_root_key).map_err(|e| {
            RemoteClientError::CertificateError(format!("failed to parse IC root key: {e}"))
        })?;
        let certified_data = client
            .execute_with_retry(|client| {
                let client = client.clone();
                async move { client.get_last_certified_block().await }
            })
            .await
            .map_err(|e| RemoteClientError::ProviderError(e.to_string()))?;

        Ok(Self {
            certified_data: CertifiedResult {
                data: did::Block::from(certified_data.data),
                certificate: certified_data.certificate,
                witness: certified_data.witness,
            },
            evmc_principal,
            ic_root_key,
        })
    }

    fn get_block_number(&self) -> u64 {
        self.certified_data.data.number.0.as_u64()
    }

    fn check_block(&self, block: &did::Block<did::Transaction>) -> Result<(), RemoteClientError> {
        if block.number < self.certified_data.data.number {
            return Ok(());
        }

        if block.number > self.certified_data.data.number {
            return Err(RemoteClientError::CertificateError(format!(
                "cannot execute block {} after the latest certified",
                block.number
            )));
        }

        if block.hash != self.certified_data.data.hash {
            return Err(RemoteClientError::CertificateError(format!(
                "state hash doesn't correspond to certified block, have {}, want {}",
                block.hash, self.certified_data.data.hash
            )));
        }

        let certificate =
            Certificate::from_cbor(&self.certified_data.certificate).map_err(|e| {
                RemoteClientError::CertificateError(format!("failed to parse certificate: {e}"))
            })?;
        certificate.verify(self.evmc_principal.as_ref(), &self.ic_root_key).map_err(|e| {
            RemoteClientError::CertificateError(format!("certificate validation error: {e}"))
        })?;

        let tree = HashTree::from_cbor(&self.certified_data.witness).map_err(|e| {
            RemoteClientError::CertificateError(format!("failed to parse witness: {e}"))
        })?;
        Self::validate_tree(self.evmc_principal.as_ref(), &certificate, &tree);

        Ok(())
    }

    fn validate_tree(canister_id: &[u8], certificate: &Certificate, tree: &HashTree) -> bool {
        let certified_data_path = [b"canister", canister_id, b"certified_data"];

        let witness = match certificate.tree.lookup_path(&certified_data_path) {
            LookupResult::Found(witness) => witness,
            _ => {
                return false;
            }
        };

        let digest = tree.digest();
        if witness != digest {
            return false;
        }

        true
    }
}

impl HeadersClient for BitfinityEvmClient {
    type Output = HeadersFut;

    fn get_headers_with_priority(
        &self,
        request: HeadersRequest,
        _priority: Priority,
    ) -> Self::Output {
        // this just searches the buffer, and fails if it can't find the header
        let mut headers = Vec::new();
        trace!(target: "downloaders::bitfinity_evm_client", request=?request, "Getting headers");

        let start_num = match request.start {
            BlockHashOrNumber::Hash(hash) => match self.hash_to_number.get(&hash) {
                Some(num) => *num,
                None => {
                    error!(%hash, "Could not find starting block number for requested header hash");
                    return Box::pin(async move { Err(RequestError::BadResponse) });
                }
            },
            BlockHashOrNumber::Number(num) => num,
        };

        let range = if request.limit == 1 {
            Either::Left(start_num..start_num + 1)
        } else {
            match request.direction {
                HeadersDirection::Rising => Either::Left(start_num..start_num + request.limit),
                HeadersDirection::Falling => {
                    Either::Right((start_num - request.limit + 1..=start_num).rev())
                }
            }
        };

        trace!(target: "downloaders::bitfinity_evm_client", range=?range, "Getting headers with range");

        for block_number in range {
            match self.headers.get(&block_number).cloned() {
                Some(header) => headers.push(header),
                None => {
                    error!(number=%block_number, "Could not find header");
                    return Box::pin(async move { Err(RequestError::BadResponse) });
                }
            }
        }

        Box::pin(async move { Ok((PeerId::default(), headers).into()) })
    }
}

impl BodiesClient for BitfinityEvmClient {
    type Output = BodiesFut;

    fn get_block_bodies_with_priority(
        &self,
        hashes: Vec<B256>,
        _priority: Priority,
    ) -> Self::Output {
        // this just searches the buffer, and fails if it can't find the block
        let mut bodies = Vec::new();

        // check if any are an error
        // could unwrap here
        for hash in hashes {
            match self.bodies.get(&hash).cloned() {
                Some(body) => bodies.push(body),
                None => {
                    error!(%hash, "Could not find body for requested block hash");
                    return Box::pin(async move { Err(RequestError::BadResponse) });
                }
            }
        }

        Box::pin(async move { Ok((PeerId::default(), bodies).into()) })
    }
}

impl DownloadClient for BitfinityEvmClient {
    fn report_bad_message(&self, _peer_id: PeerId) {
        warn!("Reported a bad message on a remote client, the client may be corrupted or invalid");
        // noop
    }

    fn num_connected_peers(&self) -> usize {
        // no such thing as connected peers when we are just using a remote
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bitfinity_remote_client_from_rpc_url() {
        let config = RpcClientConfig {
            primary_url: "https://cloudflare-eth.com".to_string(),
            backup_url: None,
            max_retries: 5,
            retry_delay: Duration::from_secs(1),
        };

        let client =
            BitfinityEvmClient::from_rpc_url(config, 0, Some(5), 5, 1000, None).await.unwrap();
        assert!(client.max_block().is_some());
    }

    #[tokio::test]
    async fn bitfinity_test_headers_client() {
        let client = BitfinityEvmClient::from_rpc_url(
            RpcClientConfig {
                primary_url: "https://cloudflare-eth.com".to_string(),
                backup_url: None,
                max_retries: 5,
                retry_delay: Duration::from_secs(1),
            },
            0,
            Some(5),
            5,
            1000,
            None,
        )
        .await
        .unwrap();
        let headers = client
            .get_headers_with_priority(
                HeadersRequest {
                    start: BlockHashOrNumber::Number(0),
                    limit: 5,
                    direction: HeadersDirection::Rising,
                },
                Priority::default(),
            )
            .await
            .unwrap();
        assert_eq!(headers.1.len(), 5);
    }

    #[tokio::test]
    async fn bitfinity_test_bodies_client() {
        let client = BitfinityEvmClient::from_rpc_url(
            RpcClientConfig {
                primary_url: "https://cloudflare-eth.com".to_string(),
                backup_url: None,
                max_retries: 5,
                retry_delay: Duration::from_secs(1),
            },
            0,
            Some(5),
            5,
            1000,
            None,
        )
        .await
        .unwrap();
        let headers = client
            .get_headers_with_priority(
                HeadersRequest {
                    start: BlockHashOrNumber::Number(0),
                    limit: 5,
                    direction: HeadersDirection::Rising,
                },
                Priority::default(),
            )
            .await
            .unwrap();
        let hashes = headers.1.iter().map(|h| h.hash_slow()).collect();
        let bodies =
            client.get_block_bodies_with_priority(hashes, Priority::default()).await.unwrap();
        assert_eq!(bodies.1.len(), 5);
    }
}
