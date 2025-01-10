//! Bitfinity block validator.

use alloy_primitives::Address;
use alloy_primitives::U256;
use did::unsafe_blocks::ValidateUnsafeBlockArgs;
use did::{Transaction, H256};
use evm_canister_client::{CanisterClient, EvmCanisterClient};
use itertools::Itertools;
use reth_chain_state::MemoryOverlayStateProvider;
use reth_evm::execute::BasicBlockExecutor;
use reth_evm::execute::BasicBlockExecutorProvider;
use reth_evm::execute::Executor as _;
use reth_evm::execute::{BlockExecutionOutput, BlockExecutorProvider as _};
use reth_evm_ethereum::execute::EthExecutionStrategy;
use reth_evm_ethereum::execute::EthExecutionStrategyFactory;
use reth_evm_ethereum::EthEvmConfig;
use reth_node_types::NodeTypesWithDB;
use reth_primitives::Receipt;
use reth_primitives::{Block, BlockWithSenders};
use reth_provider::providers::ProviderNodeTypes;
use reth_provider::HashedPostStateProvider as _;
use reth_provider::LatestStateProviderRef;
use reth_provider::{ChainSpecProvider as _, ExecutionOutcome, ProviderFactory};
use reth_revm::database::StateProviderDatabase;
use reth_trie::StateRoot;
use reth_trie_db::DatabaseStateRoot;

/// Block validator for Bitfinity.
///
/// The validator validates the block by executing it and then
/// confirming it on the EVM.
#[derive(Clone, Debug)]
pub struct BitfinityBlockValidator<C, DB>
where
    C: CanisterClient,
    DB: NodeTypesWithDB + Clone,
{
    evm_client: EvmCanisterClient<C>,
    provider_factory: ProviderFactory<DB>,
}

impl<C, DB> BitfinityBlockValidator<C, DB>
where
    C: CanisterClient,
    DB: NodeTypesWithDB<ChainSpec = reth_chainspec::ChainSpec> + ProviderNodeTypes + Clone,
{
    /// Create a new [`BitfinityBlockValidator`].
    pub fn new(evm_client: EvmCanisterClient<C>, provider_factory: ProviderFactory<DB>) -> Self {
        Self { evm_client, provider_factory }
    }

    /// Validate a block.
    pub async fn validate_block(
        &self,
        block: Block,
        transactions: &[Transaction],
    ) -> Result<(), Box<dyn std::error::Error>> {
        let validate_args = self.execute_block(block, transactions)?;
        self.validate_unsafe_block(validate_args).await?;

        Ok(())
    }

    /// Execute block and return validation arguments.
    fn execute_block(
        &self,
        block: Block,
        transactions: &[Transaction],
    ) -> Result<ValidateUnsafeBlockArgs, Box<dyn std::error::Error>> {
        // execute the block
        let block_number = block.number;
        tracing::debug!("Executing block: {block_number}",);
        let executor = self.executor();
        let block_with_senders = Self::convert_block(block, transactions);

        let output = match executor.execute((&block_with_senders, U256::MAX).into()) {
            Ok(output) => output,
            Err(err) => {
                tracing::error!("Failed to execute block: {err:?}");
                return Ok(ValidateUnsafeBlockArgs {
                    block_number,
                    block_hash: H256::zero(),
                    transactions_root: H256::zero(),
                    state_root: H256::zero(),
                    receipts_root: H256::zero(),
                });
            }
        };

        // calculate the receipts root
        let receipts_root = self.calculate_receipts_root(&output.receipts);
        tracing::debug!("Block {block_number} receipts root: {receipts_root}",);

        // calculate trasnsactions_root
        let transactions_root = self.calculate_transactions_root(&block_with_senders);
        tracing::debug!("Block {block_number} transactions root: {transactions_root}",);

        // calculate block hash
        let block_hash = self.calculate_block_hash(&block_with_senders);
        tracing::debug!("Block {block_number} hash: {block_hash}",);

        // calculate state root
        let state_root = self.calculate_state_root(output, block_number)?;
        tracing::debug!("Block {block_number} state root: {state_root}",);

        Ok(ValidateUnsafeBlockArgs {
            block_number,
            block_hash,
            transactions_root,
            state_root,
            receipts_root,
        })
    }

    /// Calculate the receipts root.
    fn calculate_receipts_root(&self, receipts: &[reth_primitives::Receipt]) -> H256 {
        let receipts_with_bloom =
            receipts.iter().map(|receipt| receipt.clone().with_bloom()).collect::<Vec<_>>();
        let calculated_root = reth_primitives::proofs::calculate_receipt_root(&receipts_with_bloom);
        H256::from_slice(calculated_root.as_ref())
    }

    /// Calculate the transactions root.
    fn calculate_transactions_root(&self, block: &BlockWithSenders) -> H256 {
        let calculated_root =
            reth_primitives::proofs::calculate_transaction_root(&block.block.body.transactions);
        H256::from_slice(calculated_root.as_ref())
    }

    /// Calculate the block hash.
    fn calculate_block_hash(&self, block: &BlockWithSenders) -> H256 {
        let calculated_hash = block.clone().seal_slow().hash();
        H256::from_slice(calculated_hash.as_ref())
    }

    /// Calculate the state root.
    fn calculate_state_root(
        &self,
        execution_output: BlockExecutionOutput<Receipt>,
        block_number: u64,
    ) -> Result<H256, Box<dyn std::error::Error>> {
        // get state root
        let execution_outcome = ExecutionOutcome::new(
            execution_output.state,
            execution_output.receipts.into(),
            block_number,
            vec![execution_output.requests.into()],
        );
        let provider = match self.provider_factory.provider() {
            Ok(provider) => provider,
            Err(err) => {
                tracing::error!("Failed to get provider: {err:?}");
                return Err(Box::new(err));
            }
        };

        let state_provider = LatestStateProviderRef::new(&provider);
        let calculated_state_root = StateRoot::overlay_root_with_updates(
            provider.tx_ref(),
            state_provider.hashed_post_state(execution_outcome.state()),
        )?
        .0;

        Ok(H256::from_slice(calculated_state_root.as_ref()))
    }

    /// Convert [`Block`] to [`BlockWithSenders`].
    fn convert_block(block: Block, transactions: &[Transaction]) -> BlockWithSenders {
        let senders = transactions
            .iter()
            .map(|tx| Address::from_slice(tx.from.0.as_ref()))
            .unique()
            .collect::<Vec<_>>();
        tracing::debug!("Found {} unique senders in block", senders.len());

        BlockWithSenders { block, senders }
    }

    /// Try to validate an unsafe block on the EVM.
    async fn validate_unsafe_block(
        &self,
        validate_args: ValidateUnsafeBlockArgs,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let block_number = validate_args.block_number;
        tracing::info!("Confirming block on EVM: {block_number}: {validate_args:?}",);

        let res = self.evm_client.validate_unsafe_block(validate_args).await?;

        tracing::info!("Validate unsafe block response for block {block_number}: {res:?}",);

        if let Err(err) = res {
            tracing::error!("Failed to validate unsafe block: {err:?}");
            return Err(Box::new(err));
        }

        Ok(())
    }

    /// Get the block executor for the latest block.
    fn executor(
        &self,
    ) -> BasicBlockExecutor<
        EthExecutionStrategy<
            StateProviderDatabase<MemoryOverlayStateProvider<reth_primitives::EthPrimitives>>,
            EthEvmConfig,
        >,
    > {
        let historical = self.provider_factory.latest().expect("no latest provider");

        let db = MemoryOverlayStateProvider::new(historical, Vec::new());
        let executor = BasicBlockExecutorProvider::new(EthExecutionStrategyFactory::ethereum(
            self.provider_factory.chain_spec(),
        ));
        executor.executor(StateProviderDatabase::new(db))
    }
}

#[cfg(test)]
mod test {

    use std::{
        collections::{BTreeMap, HashMap},
        str::FromStr,
        sync::Arc,
    };

    use alloy_consensus::TxEip1559;
    use alloy_genesis::{Genesis, GenesisAccount};
    use alloy_primitives::{FixedBytes, B256};
    use alloy_signer::SignerSync;
    use alloy_signer_local::PrivateKeySigner;
    use candid::utils::ArgumentEncoder;
    use candid::CandidType;
    use ic_canister_client::CanisterClientResult;
    use reth_chainspec::{Chain, ChainSpec, EthereumHardfork, ForkCondition, MIN_TRANSACTION_GAS};
    use reth_db::init_db;
    use reth_node_types::NodeTypes;
    use reth_primitives::{EthPrimitives, RecoveredTx, TransactionSigned};
    use reth_provider::{providers::StaticFileProvider, EthStorage};
    use reth_trie::EMPTY_ROOT_HASH;
    use reth_trie_db::MerklePatriciaTrie;
    use serde::de::DeserializeOwned;

    use super::*;

    const CHAIN_ID: u64 = 1;
    const INITIAL_BASE_FEE: u64 = 1_000_000_000;

    #[derive(Clone)]
    struct DummyDb;

    impl NodeTypesWithDB for DummyDb {
        type DB = std::sync::Arc<reth_db::DatabaseEnv>;
    }

    impl NodeTypes for DummyDb {
        type Primitives = EthPrimitives;
        type ChainSpec = ChainSpec;
        type StateCommitment = MerklePatriciaTrie;
        type Storage = EthStorage;
    }

    #[derive(Clone)]
    struct DummyCanisterClient;

    #[async_trait::async_trait]
    impl CanisterClient for DummyCanisterClient {
        async fn update<T, R>(&self, _method: &str, _args: T) -> CanisterClientResult<R>
        where
            T: ArgumentEncoder + Send + Sync,
            R: DeserializeOwned + CandidType,
        {
            unimplemented!()
        }

        async fn query<T, R>(&self, _method: &str, _args: T) -> CanisterClientResult<R>
        where
            T: ArgumentEncoder + Send + Sync,
            R: DeserializeOwned + CandidType,
        {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn test_should_validate_block() {
        let signer_pk = PrivateKeySigner::from_str(
            "cc3c029ee81d3a4122270c09ea1f4f35fed7f60c952ef0765f960de3a67f3439",
        )
        .expect("failed to parse private key");
        let signer = signer_pk.address();

        let block = generate_block(&signer_pk);
        let txs = block
            .body
            .transactions
            .iter()
            .map(|_| {
                let mut did_tx = did::Transaction::default();
                did_tx.from = signer.into();

                did_tx
            })
            .collect::<Vec<_>>();

        let client = EvmCanisterClient::new(DummyCanisterClient);

        let db_dir = tempfile::tempdir().expect("failed to create temp dir");
        let database =
            Arc::new(init_db(db_dir.path(), Default::default()).expect("failed to init db"));

        let data_dir = tempfile::tempdir().expect("failed to create temp dir");

        let chainspec = ChainSpec::builder()
            .chain(Chain::mainnet())
            .genesis(Genesis {
                alloc: BTreeMap::from([(
                    signer,
                    GenesisAccount {
                        balance: U256::from(10).pow(U256::from(18)),
                        ..Default::default()
                    },
                )]),
                ..Default::default()
            })
            .with_fork(EthereumHardfork::Frontier, ForkCondition::Block(0))
            .with_fork(EthereumHardfork::Homestead, ForkCondition::Block(0))
            .with_fork(EthereumHardfork::Shanghai, ForkCondition::Block(0))
            .with_fork(EthereumHardfork::Cancun, ForkCondition::Block(0))
            .build();
        let provider_factory: ProviderFactory<DummyDb> = ProviderFactory::new(
            database,
            Arc::new(chainspec),
            StaticFileProvider::read_write(data_dir.path()).expect("failed to create provider"),
        );
        reth_db_common::init::init_genesis(&provider_factory).expect("failed to init genesis");

        let validator = BitfinityBlockValidator::new(client, provider_factory);

        let execute_result =
            validator.execute_block(block.clone(), &txs).expect("failed to execute block");

        println!("{execute_result:?}",);

        assert_eq!(execute_result.block_number, block.header.number,);
        assert_eq!(execute_result.transactions_root, block.header.transactions_root.into());
    }

    fn generate_block(signer_pk: &PrivateKeySigner) -> reth_primitives::Block {
        let signer = signer_pk.address();
        let tx_cost = U256::from(INITIAL_BASE_FEE * MIN_TRANSACTION_GAS);
        let num_tx = 5;

        let mock_tx = |nonce: u64| -> RecoveredTx {
            let tx = reth_primitives::Transaction::Eip1559(TxEip1559 {
                chain_id: CHAIN_ID,
                nonce,
                gas_limit: MIN_TRANSACTION_GAS,
                to: signer.into(),
                max_fee_per_gas: INITIAL_BASE_FEE as u128,
                max_priority_fee_per_gas: 1,
                ..Default::default()
            });
            let signature_hash = tx.signature_hash();
            let signature = signer_pk.sign_hash_sync(&signature_hash).unwrap();

            TransactionSigned::new_unhashed(tx, signature).with_signer(signer)
        };
        let signer_balance_decrease = tx_cost * U256::from(num_tx);
        let initial_signer_balance = U256::from(10).pow(U256::from(18));
        let transactions: Vec<RecoveredTx> = (0..num_tx).map(mock_tx).collect();

        let receipts = transactions
            .iter()
            .enumerate()
            .map(|(idx, tx)| {
                Receipt {
                    tx_type: tx.tx_type(),
                    success: true,
                    cumulative_gas_used: (idx as u64 + 1) * MIN_TRANSACTION_GAS,
                    ..Default::default()
                }
                .with_bloom()
            })
            .collect::<Vec<_>>();

        let header = reth_primitives::Header {
            number: 1,
            parent_hash: FixedBytes::new([0; 32]),
            gas_used: transactions.len() as u64 * MIN_TRANSACTION_GAS,
            gas_limit: ChainSpec::default().max_gas_limit,
            mix_hash: B256::ZERO,
            base_fee_per_gas: Some(INITIAL_BASE_FEE),
            transactions_root: alloy_consensus::proofs::calculate_transaction_root(
                &transactions.clone().into_iter().map(|tx| tx.into_signed()).collect::<Vec<_>>(),
            ),
            receipts_root: alloy_consensus::proofs::calculate_receipt_root(&receipts),
            beneficiary: signer,
            state_root: reth_trie::root::state_root_unhashed(HashMap::from([(
                signer,
                (
                    reth_revm::primitives::AccountInfo {
                        balance: initial_signer_balance - signer_balance_decrease,
                        nonce: num_tx,
                        ..Default::default()
                    },
                    EMPTY_ROOT_HASH,
                ),
            )])),
            // use the number as the timestamp so it is monotonically increasing
            timestamp: 0,
            withdrawals_root: Some(alloy_consensus::proofs::calculate_withdrawals_root(&[])),
            blob_gas_used: Some(0),
            excess_blob_gas: Some(0),
            parent_beacon_block_root: None,
            ..Default::default()
        };

        reth_primitives::Block {
            header,
            body: reth_primitives::BlockBody {
                transactions: transactions.into_iter().map(|tx| tx.into_signed()).collect(),
                ommers: Vec::new(),
                withdrawals: Some(vec![].into()),
            },
        }
    }
}
