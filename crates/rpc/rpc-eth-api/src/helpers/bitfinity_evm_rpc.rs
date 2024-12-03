//! Implements Bifitnity EVM RPC methods.

use std::sync::Arc;

use alloy_rlp::Decodable;
use ethereum_json_rpc_client::{reqwest::ReqwestClient, EthJsonRpcClient};
use ethereum_json_rpc_client::{Block, CertifiedResult, Id, Params, H256};
use futures::Future;
use jsonrpsee::core::RpcResult;
use reth_chainspec::ChainSpec;
use reth_primitives::TransactionSigned;
use reth_rpc_server_types::result::{internal_rpc_err, invalid_params_rpc_err};
use reth_rpc_types::{Signature, Transaction};
use revm_primitives::{Address, Bytes, B256, U256};

use crate::RawTransactionForwarder;

/// Proxy to the Bitfinity EVM RPC.
pub trait BitfinityEvmRpc {
    /// Returns raw transactions forwarder.
    fn raw_tx_forwarder(&self) -> Option<Arc<dyn RawTransactionForwarder>> {
        None
    }

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

            Ok(U256::from(gas_price.as_u128()))
        }
    }

    /// Returns transaction from forwarder or query it from EVM RPC.
    fn transaction_by_hash(
        &self,
        hash: B256,
    ) -> impl Future<Output = RpcResult<Option<Transaction>>> + Send {
        let chain_spec = self.chain_spec();
        let forwarder = self.raw_tx_forwarder();

        async move {
            if let Some(forwarder) = forwarder {
                let tx_opt = get_transaction_from_forwarder(&*forwarder, hash).await?;
                if tx_opt.is_some() {
                    return Ok(tx_opt);
                }
            };

            // If transaction not found in forwarder, query it from EVM rpc.
            let (rpc_url, client) = get_client(&chain_spec)?;
            let method = "eth_getTransactionByHash".to_string();
            let tx: Option<Transaction> = client
                .single_request(
                    method.clone(),
                    Params::Array(vec![hash.to_string().into()]),
                    Id::Str(method),
                )
                .await
                .map_err(|e| {
                    internal_rpc_err(format!(
                        "failed to forward eth_getTransactionByHash request to {}: {}",
                        rpc_url, e
                    ))
                })?;

            Ok(tx)
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

            Ok(U256::from(priority_fee.as_u128()))
        }
    }

    /// Forwards `eth_sendRawTransaction` calls to the Bitfinity EVM
    fn send_raw_transaction(&self, tx: Bytes) -> impl Future<Output = RpcResult<B256>> + Send {
        let chain_spec = self.chain_spec();
        let forwarder = self.raw_tx_forwarder();

        async move {
            // If tx_forwarder is set, use it.
            if let Some(forwarder) = forwarder {
                let typed_tx = TransactionSigned::decode(&mut tx.as_ref()).map_err(|e| {
                    invalid_params_rpc_err(format!(
                        "failed to decode eth_sendRawTransaction input {tx}: {e}"
                    ))
                })?;
                let hash = typed_tx.hash();
                forwarder.forward_raw_transaction(&tx).await?;
                return Ok(hash);
            }

            // Otherwise, send tx directly.
            let (rpc_url, client) = get_client(&chain_spec)?;

            let tx_hash = client.send_raw_transaction_bytes(&tx).await.map_err(|e| {
                internal_rpc_err(format!(
                    "failed to forward eth_sendRawTransaction request to {}: {}",
                    rpc_url, e
                ))
            })?;

            Ok(tx_hash.0.into())
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

            Ok(balances
                .into_iter()
                .map(|(address, balance)| (address.0.into(), U256::from(balance.as_u128())))
                .collect())
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

async fn get_transaction_from_forwarder(
    forwarder: &dyn RawTransactionForwarder,
    hash: B256,
) -> RpcResult<Option<Transaction>> {
    let Some(raw) = forwarder.get_transaction_by_hash(hash).await else { return Ok(None) };
    let typed_tx = TransactionSigned::decode(&mut &raw[..])
        .map_err(|e| internal_rpc_err(format!("failed to decode transaction from bytes: {e}")))?;

    let Some(from) = typed_tx.recover_signer() else {
        return Err(internal_rpc_err("Failed to recover signer from raw transaction"));
    };

    let sig = typed_tx.signature;
    let signature =
        Signature { r: sig.r, s: sig.s, v: U256::from(sig.v(typed_tx.chain_id())), y_parity: None };

    let tx = Transaction {
        hash,
        nonce: typed_tx.nonce(),
        block_hash: None,
        block_number: None,
        transaction_index: None,
        from,
        to: typed_tx.to(),
        value: typed_tx.value(),
        gas_price: Some(typed_tx.effective_gas_price(None)),
        gas: typed_tx.gas_limit() as _,
        max_fee_per_gas: Some(typed_tx.max_fee_per_gas()),
        max_priority_fee_per_gas: typed_tx.max_priority_fee_per_gas(),
        max_fee_per_blob_gas: typed_tx.max_fee_per_blob_gas(),
        input: typed_tx.input().clone(),
        signature: Some(signature),
        chain_id: typed_tx.chain_id(),
        blob_versioned_hashes: typed_tx.blob_versioned_hashes(),
        access_list: typed_tx.access_list().cloned(),
        transaction_type: Some(typed_tx.transaction.tx_type().into()),
        other: Default::default(),
    };

    Ok(Some(tx))
}
