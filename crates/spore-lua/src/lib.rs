//! rhizome-spore-lua: Lua runtime for spore agents.
//!
//! This crate provides the Lua execution environment for agent scripts,
//! with support for integration plugins (moss, lotus, resin, etc.).

use mlua::{Lua, Result};

/// Trait for registering integration modules into the Lua runtime.
pub trait Integration {
    /// Register this integration's functions into the Lua global scope.
    fn register(&self, lua: &Lua) -> Result<()>;
}

/// The spore Lua runtime.
pub struct Runtime {
    lua: Lua,
}

impl Runtime {
    /// Create a new Lua runtime with spore-core bindings.
    pub fn new() -> Result<Self> {
        let lua = Lua::new();

        // Register spore-core bindings (LLM, memory)
        register_core(&lua)?;

        Ok(Self { lua })
    }

    /// Register an integration plugin.
    pub fn register<I: Integration>(&self, integration: &I) -> Result<()> {
        integration.register(&self.lua)
    }

    /// Run a Lua script from a string.
    pub fn run(&self, code: &str) -> Result<()> {
        self.lua.load(code).exec()
    }

    /// Run a Lua script from a file.
    pub fn run_file(&self, path: &std::path::Path) -> Result<()> {
        let code = std::fs::read_to_string(path)
            .map_err(|e| mlua::Error::external(e))?;
        self.run(&code)
    }

    /// Get a reference to the underlying Lua state.
    pub fn lua(&self) -> &Lua {
        &self.lua
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new().expect("failed to create Lua runtime")
    }
}

/// Register spore-core bindings (LLM, memory) into Lua.
fn register_core(lua: &Lua) -> Result<()> {
    let spore = lua.create_table()?;

    // TODO: Register LlmClient bindings
    // TODO: Register MemoryStore bindings

    lua.globals().set("spore", spore)?;
    Ok(())
}
