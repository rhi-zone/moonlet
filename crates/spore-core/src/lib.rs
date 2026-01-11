//! spore-core: Core runtime infrastructure.
//!
//! Shared infrastructure for spore integrations.
//! Memory store for persistent context.

mod memory;

pub use memory::{MemoryItem, MemoryStore};
