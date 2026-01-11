//! spore-core: Core runtime infrastructure.
//!
//! Shared infrastructure for spore integrations.
//! LLM functionality is behind the `llm` feature flag (to be moved to spore-llm).

#[cfg(feature = "llm")]
pub mod llm;

mod memory;

#[cfg(feature = "llm")]
pub use llm::{LlmClient, Provider};

pub use memory::{MemoryItem, MemoryStore};
