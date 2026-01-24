//! moonlet-core: Core runtime infrastructure.
//!
//! Shared infrastructure for moonlet integrations.
//! Memory store for persistent context.

mod memory;

pub use memory::{MemoryItem, MemoryStore};
