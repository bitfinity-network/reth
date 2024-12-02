mod block;

use block::BitfinityBlock;
use did::{unsafe_blocks::ValidateUnsafeBlockArgs, H256};
use evm_canister_client::EvmCanisterClient;
use ic_agent::export::Principal;
use ic_canister_client::IcAgentClient;
use reth_chainspec::{Chain, ChainSpec, EthereumHardfork, EthereumHardforks};
use reth_consensus::{Consensus, ConsensusError, PostExecutionInput};
use reth_consensus_common::validation::{
    validate_4844_header_standalone, validate_against_parent_4844,
    validate_against_parent_eip1559_base_fee, validate_against_parent_hash_number,
    validate_against_parent_timestamp, validate_block_pre_execution, validate_header_base_fee,
    validate_header_extradata, validate_header_gas,
};
use reth_primitives::{
    BlockWithSenders, Header, SealedBlock, SealedHeader, EMPTY_OMMER_ROOT_HASH, U256,
};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

use super::validation::validate_block_post_execution;

/// Ethereum beacon consensus
///
/// This consensus engine does basic checks as outlined in the execution specs.
pub struct BitfinityBeaconConsensus {
    /// Configuration
    chain_spec: Arc<ChainSpec>,
    evm_client: EvmCanisterClient<IcAgentClient>,
}

impl std::fmt::Debug for BitfinityBeaconConsensus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BitfinityBeaconConsensus").field("chain_spec", &self.chain_spec).finish()
    }
}

impl BitfinityBeaconConsensus {
    /// Create a new instance of [`BitfinityBeaconConsensus`]
    pub async fn new(
        chain_spec: Arc<ChainSpec>,
        evm_canister: Principal,
        ic_identity_path: &Path,
        network: &str,
    ) -> Self {
        let agent_client = IcAgentClient::with_identity(
            evm_canister,
            ic_identity_path,
            network,
            Some(Duration::from_secs(30)),
        )
        .await
        .expect("Failed to create agent client");

        Self { chain_spec, evm_client: EvmCanisterClient::new(agent_client) }
    }
}

#[async_trait::async_trait]
impl Consensus for BitfinityBeaconConsensus {
    fn validate_header(&self, header: &SealedHeader) -> Result<(), ConsensusError> {
        validate_header_gas(header)?;
        validate_header_base_fee(header, &self.chain_spec)?;

        // EIP-4895: Beacon chain push withdrawals as operations
        if self.chain_spec.is_shanghai_active_at_timestamp(header.timestamp)
            && header.withdrawals_root.is_none()
        {
            return Err(ConsensusError::WithdrawalsRootMissing);
        } else if !self.chain_spec.is_shanghai_active_at_timestamp(header.timestamp)
            && header.withdrawals_root.is_some()
        {
            return Err(ConsensusError::WithdrawalsRootUnexpected);
        }

        // Ensures that EIP-4844 fields are valid once cancun is active.
        if self.chain_spec.is_cancun_active_at_timestamp(header.timestamp) {
            validate_4844_header_standalone(header)?;
        } else if header.blob_gas_used.is_some() {
            return Err(ConsensusError::BlobGasUsedUnexpected);
        } else if header.excess_blob_gas.is_some() {
            return Err(ConsensusError::ExcessBlobGasUnexpected);
        } else if header.parent_beacon_block_root.is_some() {
            return Err(ConsensusError::ParentBeaconBlockRootUnexpected);
        }

        if self.chain_spec.is_prague_active_at_timestamp(header.timestamp) {
            if header.requests_root.is_none() {
                return Err(ConsensusError::RequestsRootMissing);
            }
        } else if header.requests_root.is_some() {
            return Err(ConsensusError::RequestsRootUnexpected);
        }

        Ok(())
    }

    fn validate_header_against_parent(
        &self,
        header: &SealedHeader,
        parent: &SealedHeader,
    ) -> Result<(), ConsensusError> {
        validate_against_parent_hash_number(header, parent)?;

        validate_against_parent_timestamp(header, parent)?;

        // TODO Check difficulty increment between parent and self
        // Ace age did increment it by some formula that we need to follow.
        // self.validate_against_parent_gas_limit(header, parent)?;

        validate_against_parent_eip1559_base_fee(header, parent, &self.chain_spec)?;

        // ensure that the blob gas fields for this block
        if self.chain_spec.is_cancun_active_at_timestamp(header.timestamp) {
            validate_against_parent_4844(header, parent)?;
        }

        Ok(())
    }

    fn validate_header_with_total_difficulty(
        &self,
        header: &Header,
        total_difficulty: U256,
    ) -> Result<(), ConsensusError> {
        let is_post_merge = self
            .chain_spec
            .fork(EthereumHardfork::Paris)
            .active_at_ttd(total_difficulty, header.difficulty);

        if is_post_merge {
            if !header.is_zero_difficulty() {
                return Err(ConsensusError::TheMergeDifficultyIsNotZero);
            }

            if header.nonce != 0 {
                return Err(ConsensusError::TheMergeNonceIsNotZero);
            }

            if header.ommers_hash != EMPTY_OMMER_ROOT_HASH {
                return Err(ConsensusError::TheMergeOmmerRootIsNotEmpty);
            }

            // Post-merge, the consensus layer is expected to perform checks such that the block
            // timestamp is a function of the slot. This is different from pre-merge, where blocks
            // are only allowed to be in the future (compared to the system's clock) by a certain
            // threshold.
            //
            // Block validation with respect to the parent should ensure that the block timestamp
            // is greater than its parent timestamp.

            // validate header extradata for all networks post merge
            validate_header_extradata(header)?;

            // mixHash is used instead of difficulty inside EVM
            // https://eips.ethereum.org/EIPS/eip-4399#using-mixhash-field-instead-of-difficulty
        } else {
            // TODO Consensus checks for old blocks:
            //  * difficulty, mix_hash & nonce aka PoW stuff
            // low priority as syncing is done in reverse order

            // Check if timestamp is in the future. Clock can drift but this can be consensus issue.
            let present_timestamp =
                SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();

            if header.exceeds_allowed_future_timestamp(present_timestamp) {
                return Err(ConsensusError::TimestampIsInFuture {
                    timestamp: header.timestamp,
                    present_timestamp,
                });
            }

            // Goerli and early OP exception:
            //  * If the network is goerli pre-merge, ignore the extradata check, since we do not
            //  support clique. Same goes for OP blocks below Bedrock.
            if self.chain_spec.chain != Chain::goerli() && !self.chain_spec.is_optimism() {
                validate_header_extradata(header)?;
            }
        }

        Ok(())
    }

    fn validate_block_pre_execution(&self, block: &SealedBlock) -> Result<(), ConsensusError> {
        validate_block_pre_execution(block, &self.chain_spec)
    }

    fn validate_block_post_execution(
        &self,
        block: &BlockWithSenders,
        input: PostExecutionInput<'_>,
    ) -> Result<(), ConsensusError> {
        validate_block_post_execution(block, &self.chain_spec, input.receipts, input.requests)
    }

    /// Confirm a block on the EVM.
    ///
    /// Used only for engines which communicates with the EVM to confirm blocks.
    async fn confirm_block_on_evm(
        &self,
        block: &BlockWithSenders,
        input: PostExecutionInput<'_>,
    ) -> Result<(), ConsensusError> {
        let block = BitfinityBlock::from(block);

        let res = self
            .evm_client
            .validate_unsafe_block(ValidateUnsafeBlockArgs {
                block_number: block.number(),
                block_hash: block.hash(),
                state_root: block.state_root(),
                transactions_root: block.transaction_root(),
                receipts_root: block.receipts_root(&input.receipts),
            })
            .await
            .map_err(|_| ConsensusError::RequestsRootUnexpected)?;

        tracing::info!(
            "Validate unsafe block response for block {} (confirm): {res:?}",
            block.number()
        );

        if let Err(err) = res {
            tracing::error!("Failed to validate unsafe block: {err:?}");
            return Err(ConsensusError::RequestsRootUnexpected);
        }

        Ok(())
    }

    /// Reject a block on the EVM, by sending an invalid block to the EVM.
    async fn reject_block_on_evm(&self, block: &SealedBlock) -> Result<(), ConsensusError> {
        // we expect error here
        let res = self
            .evm_client
            .validate_unsafe_block(ValidateUnsafeBlockArgs {
                block_number: block.number,
                block_hash: H256::zero(),
                state_root: H256::zero(),
                transactions_root: H256::zero(),
                receipts_root: H256::zero(),
            })
            .await
            .map_err(|_| ConsensusError::RequestsRootUnexpected)?;

        if res.is_ok() {
            return Err(ConsensusError::RequestsRootUnexpected);
        }

        tracing::info!(
            "Validate unsafe block response for block {} (reject): {res:?}",
            block.number
        );

        Ok(())
    }
}
