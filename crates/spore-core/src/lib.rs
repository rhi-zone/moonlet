//! spore-core: Agentic AI infrastructure.
//!
//! This crate provides core building blocks for AI agents:
//! - Multi-provider LLM client (Anthropic, OpenAI, Gemini, etc.)
//! - SQLite-backed memory store for context persistence

#[cfg(feature = "llm")]
pub mod llm;

mod memory;

#[cfg(feature = "llm")]
pub use llm::{LlmClient, Provider};

pub use memory::{MemoryItem, MemoryStore};
