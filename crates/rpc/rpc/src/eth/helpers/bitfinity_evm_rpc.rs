//! Implements Bifitnity EVM RPC methods.
//!
use std::sync::Arc;

<<<<<<< HEAD
use reth_provider::ChainSpecProvider;
use reth_rpc_eth_api::{helpers::bitfinity_evm_rpc::BitfinityEvmRpc, RawTransactionForwarder};
||||||| 0e2237228
use reth_provider::ChainSpecProvider;
use reth_rpc_eth_api::helpers::bitfinity_evm_rpc::BitfinityEvmRpc;
=======
use reth_provider::{BlockReader, ChainSpecProvider};
use reth_rpc_eth_api::helpers::bitfinity_evm_rpc::BitfinityEvmRpc;
>>>>>>> bitfinity-archive-node

use crate::EthApi;

impl<Provider, Pool, Network, EvmConfig> BitfinityEvmRpc
    for EthApi<Provider, Pool, Network, EvmConfig>
where
<<<<<<< HEAD
    Provider: ChainSpecProvider,
{
    fn raw_tx_forwarder(&self) -> Option<Arc<dyn RawTransactionForwarder>> {
        self.inner.raw_tx_forwarder()
    }

||||||| 0e2237228

impl<Provider, Pool, Network, EvmConfig> BitfinityEvmRpc for EthApi<Provider, Pool, Network, EvmConfig> 
where Provider: ChainSpecProvider {

=======
    Provider: BlockReader + ChainSpecProvider<ChainSpec = reth_chainspec::ChainSpec>,
{
>>>>>>> bitfinity-archive-node
    fn chain_spec(&self) -> Arc<reth_chainspec::ChainSpec> {
        self.inner.provider().chain_spec()
    }
}
