//! Implements Bifitnity EVM RPC methods.

use std::sync::Arc;

use alloy_network::TransactionResponse;
use alloy_rlp::Decodable;
use did::{Block, H256};
use ethereum_json_rpc_client::CertifiedResult;
use ethereum_json_rpc_client::{reqwest::ReqwestClient, EthJsonRpcClient};
use futures::Future;
use jsonrpsee::core::RpcResult;
use reth_chainspec::ChainSpec;
use reth_primitives::{RecoveredTx, TransactionSigned};
use reth_primitives_traits::SignedTransaction;
use reth_rpc_eth_types::TransactionSource;
use reth_rpc_server_types::result::{internal_rpc_err, invalid_params_rpc_err};
use revm_primitives::{Address, Bytes, B256, U256};

use super::bitfinity_tx_forwarder::BitfinityTransactionsForwarder;

/// Proxy to the Bitfinity EVM RPC.
pub trait BitfinityEvmRpc {
    /// Transaction type.
    type Transaction: SignedTransaction;

    /// Returns transaction forwarder.
    fn bitfinity_transaction_forwarder(&self) -> Option<BitfinityTransactionsForwarder>;

    /// Returns the `ChainSpec`.
    fn chain_spec(&self) -> Arc<ChainSpec>;

    /// Forwards `eth_gasPrice` calls to the Bitfinity EVM.
    fn btf_gas_price(&self) -> impl Future<Output = RpcResult<U256>> + Send {
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

    /// Returns transaction from forwarder or query it from EVM RPC.
    fn btf_transaction_by_hash(
        &self,
        hash: B256,
    ) -> impl Future<Output = RpcResult<Option<TransactionSource<Self::Transaction>>>> + Send {
        let chain_spec = self.chain_spec();
        let forwarder = self.bitfinity_transaction_forwarder();

        async move {
            // Try to find transaction in transaction forwarder.
            if let Some(forwarder) = forwarder {
                if let Some(raw_tx) = forwarder.get_transaction_by_hash(hash).await {
                    let self_tx =
                        <Self::Transaction as alloy_rlp::Decodable>::decode(&mut raw_tx.as_ref())
                            .map_err(|e| {
                            internal_rpc_err(format!(
                            "failed to decode BitfinityEvmRpc::Transaction from tx forwarder: {e}"
                        ))
                        })?;

                    let signer = self_tx.recover_signer().ok_or_else(|| {
                        internal_rpc_err(
                            "failed to recover signer from decoded BitfinityEvmRpc::Transaction",
                        )
                    })?;
                    let recovered_tx = RecoveredTx::new_unchecked(self_tx, signer);
                    return Ok(Some(TransactionSource::Pool(recovered_tx)));
                }
            };

            // If transaction is not found in forwarder, query it from EVM rpc.
            let (rpc_url, client) = get_client(&chain_spec)?;
            let Some(tx) = client.get_transaction_by_hash(hash.into()).await.map_err(|e| {
                internal_rpc_err(format!(
                    "failed to forward eth_transactionByHash request to {}: {}",
                    rpc_url, e
                ))
            })?
            else {
                return Ok(None);
            };

            let alloy_tx: alloy_rpc_types_eth::Transaction = tx.try_into().map_err(|e| {
                internal_rpc_err(format!(
                    "failed to convert did::Transaction into alloy_rpc_types::Transaction: {e}"
                ))
            })?;
            let encoded = alloy_rlp::encode(&alloy_tx.inner);
            let self_tx =
                <Self::Transaction as alloy_rlp::Decodable>::decode(&mut encoded.as_ref())
                    .map_err(|e| internal_rpc_err(format!("failed to decode BitfinityEvmRpc::Transaction from received did::Transaction: {e}")))?;

            let signer = self_tx.recover_signer().ok_or_else(|| {
                internal_rpc_err(
                    "failed to recover signer from decoded BitfinityEvmRpc::Transaction",
                )
            })?;
            let recovered_tx = RecoveredTx::new_unchecked(self_tx, signer);

            let block_params = alloy_tx
                .block_number()
                .zip(alloy_tx.transaction_index())
                .zip(alloy_tx.block_hash());
            let tx_source = match block_params {
                Some(((block_number, index), block_hash)) => TransactionSource::Block {
                    transaction: recovered_tx,
                    index,
                    block_hash,
                    block_number,
                    base_fee: None,
                },
                None => TransactionSource::Pool(recovered_tx),
            };

            Ok(Some(tx_source))
        }
    }

    /// Forwards `eth_maxPriorityFeePerGas` calls to the Bitfinity EVM
    fn btf_max_priority_fee_per_gas(&self) -> impl Future<Output = RpcResult<U256>> + Send {
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
    fn btf_send_raw_transaction(&self, tx: Bytes) -> impl Future<Output = RpcResult<B256>> + Send {
        let chain_spec = self.chain_spec();
        let forwarder = self.bitfinity_transaction_forwarder();

        async move {
            if let Some(forwarder) = forwarder {
                let typed_tx = TransactionSigned::decode(&mut tx.as_ref()).map_err(|e| {
                    invalid_params_rpc_err(format!(
                        "failed to decode eth_sendRawTransaction input {tx}: {e}"
                    ))
                })?;
                let hash = typed_tx.hash();
                forwarder.forward_raw_transaction(&tx).await?;
                return Ok(hash);
            };

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
