//! Bitfinity block validator.

use did::unsafe_blocks::ValidateUnsafeBlockArgs;
use did::{Transaction, H256};
use evm_canister_client::{CanisterClient, EvmCanisterClient};
use itertools::Itertools;
use reth_db::database::Database;
use reth_engine_tree::tree::MemoryOverlayStateProvider;
use reth_evm::execute::BlockExecutorProvider as _;
use reth_evm::execute::Executor as _;
use reth_evm_ethereum::execute::EthExecutorProvider;
use reth_evm_ethereum::{execute::EthBlockExecutor, EthEvmConfig};
use reth_primitives::Address;
use reth_primitives::U256;
use reth_primitives::{Block, BlockWithSenders};
use reth_provider::{ChainSpecProvider as _, ExecutionOutcome, ProviderFactory, StateProvider};
use reth_revm::database::StateProviderDatabase;

/// Block validator for Bitfinity.
///
/// The validator validates the block by executing it and then
/// confirming it on the EVM.
#[derive(Clone, Debug)]
pub struct BitfinityBlockValidator<C, DB>
where
    C: CanisterClient,
{
    evm_client: EvmCanisterClient<C>,
    provider_factory: ProviderFactory<DB>,
}

impl<C, DB> BitfinityBlockValidator<C, DB>
where
    C: CanisterClient,
    DB: Database,
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
        let res = self.evm_client.is_unsafe_blocks_enabled().await?;

        Ok(res)
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
        let receipts_with_bloom =
            output.receipts.iter().map(|receipt| receipt.clone().with_bloom()).collect::<Vec<_>>();
        let calculated_root = reth_primitives::proofs::calculate_receipt_root(&receipts_with_bloom);
        let receipts_root = H256::from_slice(calculated_root.as_ref());
        tracing::debug!("Block {block_number} receipts root: {receipts_root}",);

        // calculate trasnsactions_root
        let calculated_root =
            reth_primitives::proofs::calculate_transaction_root(&block_with_senders.body);
        let transactions_root = H256::from_slice(calculated_root.as_ref());
        tracing::debug!("Block {block_number} transactions root: {transactions_root}",);

        // calculate block hash
        let calculated_block_hash = block_with_senders.clone().seal_slow().hash();
        let block_hash = H256::from_slice(calculated_block_hash.as_ref());
        tracing::debug!("Block {block_number} hash: {block_hash}",);

        // get state root
        let execution_outcome = ExecutionOutcome::new(
            output.state,
            output.receipts.into(),
            block_number,
            vec![output.requests.into()],
        );
        let provider = match self.provider_factory.provider() {
            Ok(provider) => provider,
            Err(err) => {
                tracing::error!("Failed to get provider: {err:?}");
                return Err(Box::new(err));
            }
        };
        let calculated_state_root =
            execution_outcome.hash_state_slow().state_root_with_updates(provider.tx_ref())?.0;
        let state_root = H256::from_slice(calculated_state_root.as_ref());
        tracing::debug!("Block {block_number} state root: {state_root}",);

        Ok(ValidateUnsafeBlockArgs {
            block_number,
            block_hash,
            transactions_root,
            state_root,
            receipts_root,
        })
    }

    /// Convert [`Block`] to [`BlockWithSenders`].
    fn convert_block(block: Block, transactions: &[Transaction]) -> BlockWithSenders {
        let senders = transactions
            .iter()
            .map(|tx| &tx.from)
            .map(|from| Address::from_slice(from.0.as_ref()))
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
    ) -> EthBlockExecutor<
        EthEvmConfig,
        StateProviderDatabase<MemoryOverlayStateProvider<Box<dyn StateProvider>>>,
    > {
        let historical = self.provider_factory.latest().expect("no latest provider");

        let db = MemoryOverlayStateProvider::new(Vec::new(), historical);
        let executor = EthExecutorProvider::ethereum(self.provider_factory.chain_spec());
        executor.executor(StateProviderDatabase::new(db))
    }
}
