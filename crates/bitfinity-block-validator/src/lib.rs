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
        if !self.unsafe_blocks_enabled().await? {
            tracing::debug!("Unsafe blocks are disabled");
            return Ok(());
        }
        let validate_args = self.execute_block(block, transactions)?;
        self.validate_unsafe_block(validate_args).await?;

        Ok(())
    }

    /// Get whether unsafe blocks are enabled.
    async fn unsafe_blocks_enabled(&self) -> Result<bool, Box<dyn std::error::Error>> {
        self.evm_client
            .is_unsafe_blocks_enabled()
            .await
            .map_err(|err| Box::new(err) as Box<dyn std::error::Error>)
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
