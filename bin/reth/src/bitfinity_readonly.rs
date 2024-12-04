//! Bitfinity Readonly Node

use std::hash::Hash;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;

use reth::blockchain_tree::BlockchainTreeEngine;
use reth_db::DatabaseEnv;
use reth_errors::ProviderError;
use reth_provider::providers::BlockchainProvider;
use reth_provider::{
    BlockNumReader, BlockReader, CanonChainTracker, DatabaseProviderFactory, HeaderProvider,
    HeaderSyncGapProvider,
};
use reth_stages::Pipeline;
// We use jemalloc for performance reasons.
#[cfg(all(feature = "jemalloc", unix))]
#[global_allocator]
static ALLOC: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(all(feature = "optimism", not(test)))]
compile_error!("Cannot build the `reth` binary with the `optimism` feature flag enabled. Did you mean to build `op-reth`?");

#[cfg(not(feature = "optimism"))]
fn main() {
    use lightspeed_scheduler::job::Job;
    use lightspeed_scheduler::scheduler::Scheduler;
    use lightspeed_scheduler::JobExecutor;
    use reth::cli::Cli;

    use reth_node_ethereum::EthereumNode;

    reth::sigsegv_handler::install();

    // Enable backtraces unless a RUST_BACKTRACE value has already been explicitly provided.
    if std::env::var_os("RUST_BACKTRACE").is_none() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    if let Err(err) = Cli::parse_args().run(|builder, _| async {
        let handle = builder.launch_readonly_node(EthereumNode::default()).await?;

        // Schedule for refetching the chain info
        let job_executor = JobExecutor::new_with_local_tz();

        // Schedule the import job
        {
            let interval = Duration::from_secs(5);
            job_executor
                .add_job_with_scheduler(
                    Scheduler::Interval { interval_duration: interval, execute_at_startup: false },
                    Job::new("update_chain_info", "update chain info", None, move || {
                        let blockchain_provider = handle.node.provider.clone();
                        Box::pin(async move {
                            start_chain_info_updater(blockchain_provider)
                                .expect("Failed to update chain info");

                            Ok(())
                        })
                    }),
                )
                .await;
        }

        let _job_executor = job_executor.run().await.expect("Failed to run job executor");

        handle.node_exit_future.await
    }) {
        eprintln!("Error: {err:?}");
        std::process::exit(1);
    }
}

fn start_chain_info_updater(
    blockchain_provider: BlockchainProvider<Arc<DatabaseEnv>>,
) -> eyre::Result<()> {
    // Get a read-only database provider
    let provider = blockchain_provider.database_provider_ro()?;

    // Get the current chain info from database
    let chain_info = provider.chain_info()?;

    // Get the current provider's chain info
    let current_info = blockchain_provider.chain_info()?;

    // Only update if the database has a newer block
    if chain_info.best_number > current_info.best_number {
        println!(
            "Found new blocks: current={} new={}",
            current_info.best_number, chain_info.best_number
        );

        // First, update the block hashes and clear any buffered blocks
        if let Ok(updated_hashes) = blockchain_provider.update_block_hashes_and_clear_buffered() {
            println!("Updated block hashes in the tree");

            // Now walk through all new blocks and update the provider's state
            for block_number in (current_info.best_number + 1)..=chain_info.best_number {
                if let Ok(Some(header)) = provider.header_by_number(block_number) {
                    let block_hash = header.hash_slow();

                    // Try to get all the block data
                    match (provider.block(block_hash), provider.receipt(block_number)) {
                        (Ok(Some(block)), Ok(Some(receipts))) => {
                            println!(
                                "Processing block {}: hash={:?} txns={}",
                                block_number,
                                block_hash,
                                block.body.len()
                            );

                            // Get the sealed header for chain info updates
                            if let Ok(Some(sealed_header)) = provider.sealed_header(block_number) {
                                // Update the chain info tracker
                                blockchain_provider.set_canonical_head(sealed_header.clone());
                                blockchain_provider.set_finalized(sealed_header.clone());
                                blockchain_provider.set_safe(sealed_header);

                                // Get a read-write provider to update the database view
                                if let Ok(mut provider_rw) =
                                    blockchain_provider.database_provider_rw()
                                {
                                    // Update block tables
                                    if let Err(e) = provider_rw.insert_block(block.clone(), None) {
                                        println!(
                                            "Failed to insert block {}: {:?}",
                                            block_number, e
                                        );
                                        continue;
                                    }

                                    // Update receipts
                                    if let Err(e) =
                                        provider_rw.insert_receipts(block_number, receipts)
                                    {
                                        println!(
                                            "Failed to insert receipts for block {}: {:?}",
                                            block_number, e
                                        );
                                        continue;
                                    }

                                    // Commit changes
                                    if let Err(e) = provider_rw.commit() {
                                        println!(
                                            "Failed to commit changes for block {}: {:?}",
                                            block_number, e
                                        );
                                        continue;
                                    }

                                    println!(
                                        "Successfully updated state for block {}",
                                        block_number
                                    );
                                }
                            }
                        }
                        _ => {
                            println!("Incomplete data for block {}, skipping", block_number);
                            continue;
                        }
                    }
                }
            }

            // Verify the final state
            if let Ok(final_info) = blockchain_provider.chain_info() {
                println!(
                    "Final provider state: number={} hash={:?}",
                    final_info.best_number, final_info.best_hash
                );
            }
        }
    }

    Ok(())
}
