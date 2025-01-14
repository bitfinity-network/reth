//! Implements Bifitnity EVM RPC methods.
//!
use std::sync::Arc;

use reth_provider::{BlockReader, ChainSpecProvider};
use reth_rpc_eth_api::helpers::{
    bitfinity_evm_rpc::BitfinityEvmRpc, bitfinity_tx_forwarder::BitfinityTransactionsForwarder,
};

use crate::EthApi;

impl<Provider, Pool, Network, EvmConfig> BitfinityEvmRpc
    for EthApi<Provider, Pool, Network, EvmConfig>
where
    Provider: BlockReader + ChainSpecProvider<ChainSpec = reth_chainspec::ChainSpec>,
{
    type Transaction = Provider::Transaction;

    fn bitfinity_transaction_forwarder(&self) -> Option<BitfinityTransactionsForwarder> {
        self.bitfinity_tx_forwarder.get().cloned()
    }

    fn chain_spec(&self) -> Arc<reth_chainspec::ChainSpec> {
        self.inner.provider().chain_spec()
    }
}
