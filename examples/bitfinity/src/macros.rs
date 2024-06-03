//! Helper macros

use std::path::PathBuf;

use eyre::Context;
use reth_config::Config;

/// Creates the block executor type based on the configured feature.
///
/// Note(mattsse): This is incredibly horrible and will be replaced
#[cfg(not(feature = "optimism"))]
#[macro_export]
macro_rules! block_executor {
    ($chain_spec:expr) => {
        reth_node_ethereum::EthExecutorProvider::ethereum($chain_spec)
    };
}
