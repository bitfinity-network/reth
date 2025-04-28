/// Bitfinity specific configuration
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BitfinitySpec {
    /// URL of the Bitfinity EVM node
    pub rpc_url: String,
    /// Send transaction to the Bitfinity EVM node
    pub send_transaction_url: Option<String>,
}
