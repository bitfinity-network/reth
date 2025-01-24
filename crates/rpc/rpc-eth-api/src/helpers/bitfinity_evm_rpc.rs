//! Implements Bifitnity EVM RPC methods.

use std::sync::Arc;

use alloy_consensus::Transaction;
use alloy_rlp::Decodable;
use did::{Block, H256};
use ethereum_json_rpc_client::CertifiedResult;
use ethereum_json_rpc_client::{reqwest::ReqwestClient, EthJsonRpcClient};
use futures::Future;
use jsonrpsee::core::RpcResult;
use reth_chainspec::{ChainSpec, EthChainSpec};
use reth_primitives::TransactionSigned;
use reth_primitives_traits::constants::MINIMUM_GAS_LIMIT;
use reth_primitives_traits::SignedTransaction;
use reth_rpc_server_types::result::{internal_rpc_err, invalid_params_rpc_err};
use revm_primitives::{Address, Bytes, B256, U256};

/// Proxy to the Bitfinity EVM RPC.
pub trait BitfinityEvmRpc {
    /// Returns the `ChainSpec`.
    fn chain_spec(&self) -> Arc<ChainSpec>;

    /// Forwards `eth_gasPrice` calls to the Bitfinity EVM.
    fn gas_price(&self) -> impl Future<Output = RpcResult<U256>> + Send {
        let chain_spec = self.chain_spec();
        async move {
            let (rpc_url, client) = get_client(&chain_spec)?;

            let gas_price = client.gas_price().await.map_err(|e| {
                internal_rpc_err(format!(
                    "failed to forward eth_gasPrice request to {}: {}",
                    rpc_url, e
                ))
            })?;

            Ok(gas_price.0)
        }
    }

    /// Forwards `eth_maxPriorityFeePerGas` calls to the Bitfinity EVM
    fn max_priority_fee_per_gas(&self) -> impl Future<Output = RpcResult<U256>> + Send {
        let chain_spec = self.chain_spec();
        async move {
            let (rpc_url, client) = get_client(&chain_spec)?;

            let priority_fee = client.max_priority_fee_per_gas().await.map_err(|e| {
                internal_rpc_err(format!(
                    "failed to forward eth_maxPriorityFeePerGas request to {}: {}",
                    rpc_url, e
                ))
            })?;

            Ok(priority_fee.0)
        }
    }

    /// Forwards `eth_sendRawTransaction` calls to the Bitfinity EVM
    fn send_raw_transaction(&self, tx: Bytes) -> impl Future<Output = RpcResult<B256>> + Send {
        let chain_spec = self.chain_spec();
        async move {
            let typed_tx = TransactionSigned::decode(&mut tx.as_ref()).map_err(|e| {
                invalid_params_rpc_err(format!(
                    "failed to decode eth_sendRawTransaction input {tx}: {e}"
                ))
            })?;

            validate_raw_transaction(&typed_tx, &chain_spec)?;

            let (rpc_url, client) = get_client(&chain_spec)?;

            let tx_hash = client.send_raw_transaction_bytes(&tx).await.map_err(|e| {
                internal_rpc_err(format!(
                    "failed to forward eth_sendRawTransaction request to {}: {}",
                    rpc_url, e
                ))
            })?;

            Ok(tx_hash.0)
        }
    }

    /// Forwards `ic_getGenesisBalances` calls to the Bitfinity EVM
    fn get_genesis_balances(&self) -> impl Future<Output = RpcResult<Vec<(Address, U256)>>> + Send {
        let chain_spec = self.chain_spec();
        async move {
            let (rpc_url, client) = get_client(&chain_spec)?;

            let balances = client.get_genesis_balances().await.map_err(|e| {
                internal_rpc_err(format!(
                    "failed to forward ic_getGenesisBalances request to {}: {}",
                    rpc_url, e
                ))
            })?;

            Ok(balances.into_iter().map(|(address, balance)| (address.0, balance.0)).collect())
        }
    }

    /// Forwards `ic_getLastCertifiedBlock` calls to the Bitfinity EVM
    fn get_last_certified_block(
        &self,
    ) -> impl Future<Output = RpcResult<CertifiedResult<Block<H256>>>> + Send {
        let chain_spec = self.chain_spec();
        async move {
            let (rpc_url, client) = get_client(&chain_spec)?;

            let certified_block = client.get_last_certified_block().await.map_err(|e| {
                internal_rpc_err(format!(
                    "failed to forward get_last_certified_block request to {}: {}",
                    rpc_url, e
                ))
            })?;

            Ok(certified_block)
        }
    }
}

/// Returns a client for the Bitfinity EVM RPC.
fn get_client(chain_spec: &ChainSpec) -> RpcResult<(&String, EthJsonRpcClient<ReqwestClient>)> {
    let Some(rpc_url) = &chain_spec.bitfinity_evm_url else {
        return Err(internal_rpc_err("bitfinity_evm_url not found in chain spec"));
    };

    let client = ethereum_json_rpc_client::EthJsonRpcClient::new(
        ethereum_json_rpc_client::reqwest::ReqwestClient::new(rpc_url.to_string()),
    );

    Ok((rpc_url, client))
}

/// Validates:
/// - chain id
/// - signature
/// - gas limit
fn validate_raw_transaction(tx: &TransactionSigned, chain_spec: &ChainSpec) -> RpcResult<()> {
    // Check chain id
    if tx.chain_id() != Some(chain_spec.chain_id()) {
        return Err(invalid_params_rpc_err(format!(
            "expected chain id == {}",
            chain_spec.chain_id()
        )));
    }

    // Check signature correctness
    if tx.recover_signer().is_none() {
        return Err(invalid_params_rpc_err(
            "transaction signature verification failed".to_string(),
        ));
    }

    // Check signature malleability
    did::transaction::Signature::check_malleability(&tx.signature.s().into())
        .map_err(|e| invalid_params_rpc_err(format!("signature malleability check failed: {e}")))?;

    // Check min gas limit
    if tx.gas_limit() < MINIMUM_GAS_LIMIT {
        return Err(invalid_params_rpc_err(format!(
            "expected gas limit greater or equal to {MINIMUM_GAS_LIMIT}, found: {}",
            tx.gas_limit()
        )));
    }

    Ok(())
}
