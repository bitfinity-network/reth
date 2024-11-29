#![allow(missing_docs)]

use std::{sync::Arc, time::Duration};

use reth::bitfinity_tasks::send_txs::{
    BitfinityTransactionSender, BitfinityTransactionsForwarder, TransactionsPriorityQueue,
};
use tokio::sync::Mutex;

// We use jemalloc for performance reasons.
#[cfg(all(feature = "jemalloc", unix))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(all(feature = "optimism", not(test)))]
compile_error!("Cannot build the `reth` binary with the `optimism` feature flag enabled. Did you mean to build `op-reth`?");

#[cfg(not(feature = "optimism"))]
fn main() {
    use reth::cli::Cli;
    use reth::commands::bitfinity_import::BitfinityImportCommand;
    use reth_node_ethereum::EthereumNode;

    reth::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    let queue = Arc::new(Mutex::new(TransactionsPriorityQueue::new(1000)));
    let queue_clone = Arc::clone(&queue);

    if let Err(err) = Cli::parse_args().run(move |builder, _| async {
        let handle = builder
            .node(EthereumNode::default())
            .extend_rpc_modules(move |ctx| {
                let forwarder_required = ctx.config().bitfinity_import_arg.tx_queue;
                if forwarder_required {
                    // Add custom forwarder with transactions priority queue.
                    queue
                        .blocking_lock()
                        .set_size_limit(ctx.config().bitfinity_import_arg.tx_queue_size);

                    let forwarder = BitfinityTransactionsForwarder::new(queue);
                    ctx.registry.set_eth_raw_transaction_forwarder(Arc::new(forwarder));
                }

                Ok(())
            })
            .launch()
            .await?;

        let blockchain_provider = handle.node.provider.clone();
        let config = handle.node.config.config.clone();
        let chain_spec = handle.node.chain_spec();
        let datadir = handle.node.data_dir.clone();
        let (provider_factory, bitfinity) =
            handle.bitfinity_import.clone().expect("Bitfinity import not configured");

        // Init bitfinity import
        let executor = {
            let import = BitfinityImportCommand::new(
                config,
                datadir,
                chain_spec.clone(),
                bitfinity.clone(),
                provider_factory,
                blockchain_provider,
            );
            let (executor, _job_handle) = import.schedule_execution(None).await?;
            executor
        };

        if bitfinity.tx_queue {
            let url = bitfinity.send_raw_transaction_rpc_url.unwrap_or(bitfinity.rpc_url);

            // Init transaction sending cycle.
            let period = Duration::from_secs(bitfinity.send_queued_txs_period_secs);
            let transaction_sending = BitfinityTransactionSender::new(
                queue_clone,
                url,
                period,
                bitfinity.queued_txs_batch_size,
                bitfinity.queued_txs_per_execution_threshold,
            );
            let _sending_handle = transaction_sending.schedule_execution(Some(executor)).await?;
        }

        handle.node_exit_future.await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}
