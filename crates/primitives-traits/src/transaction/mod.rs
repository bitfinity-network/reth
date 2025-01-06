//! Transaction abstraction

pub mod execute;
pub mod signed;
pub mod tx_type;

use crate::{InMemorySize, MaybeArbitrary, MaybeCompact, MaybeSerde};
use core::{fmt, hash::Hash};

use alloy_consensus::constants::{
    EIP1559_TX_TYPE_ID, EIP2930_TX_TYPE_ID, EIP4844_TX_TYPE_ID, EIP7702_TX_TYPE_ID,
    LEGACY_TX_TYPE_ID,
};

/// Helper trait that unifies all behaviour required by transaction to support full node operations.
pub trait FullTransaction: Transaction + MaybeCompact {}

impl<T> FullTransaction for T where T: Transaction + MaybeCompact {}

/// Abstraction of a transaction.
pub trait Transaction:
    Send
    + Sync
    + Unpin
    + Clone
    + fmt::Debug
    + Eq
    + PartialEq
    + Hash
    + alloy_consensus::Transaction
    + InMemorySize
    + MaybeSerde
    + MaybeArbitrary
{
}

impl<T> Transaction for T where
    T: Send
        + Sync
        + Unpin
        + Clone
        + fmt::Debug
        + Eq
        + PartialEq
        + Hash
        + alloy_consensus::Transaction
        + InMemorySize
        + MaybeSerde
        + MaybeArbitrary
{
}
