use std::sync::Arc;

use async_trait::async_trait;
use clap::Parser;
use futures::{Future, StreamExt};
use reth::builder::{NodeBuilder, WithLaunchContext};
use reth::network::NetworkInfo;
use reth::revm::DatabaseCommit;
use reth::rpc::api::EthApiServer;
use reth::rpc::eth::{EthApi, EthApiSpec, EthTransactions};
use reth::transaction_pool::TransactionPool;
use reth_db::database::Database;
use reth_db::DatabaseEnv;
use reth_exex::ExExContext;
use reth_node_api::{ConfigureEvm, FullNodeComponents};
use reth_provider::{
    BlockIdReader, BlockNumReader, BlockReader, BlockReaderIdExt, ChainSpecProvider,
    EvmEnvProvider, HeaderProvider, ProviderFactory, StateProviderFactory,
};

use crate::args::BitfinityArgs;
use crate::import::BitfinityImportCommand;

mod args;
mod bitfinity_client;
mod import;
mod macros;

fn main() {
    use reth::cli::Cli;
    use reth_node_ethereum::EthereumNode;

    reth::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    if let Err(err) = Cli::parse().run(|builder, bitfinity_args: BitfinityArgs| async {
        let mut handle = builder.launch_node(EthereumNode::default()).await?;

        let blockchain_provider = handle.node.provider.clone();
        let config = handle.node.config.config.clone();
        let chain = handle.node.chain_spec().clone();
        let datadir = handle.node.data_dir.clone();

        // Init bitfinity import
        {
            let import = BitfinityImportCommand::new(
                config,
                datadir,
                chain,
                bitfinity_args,
                blockchain_provider,
            );
            let _import_handle = import.execute().await?;
        }

        let traceapi = handle.node.;

        handle.node_exit_future.await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
