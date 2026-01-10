//! rhizome-spore-moss: Moss integration for spore agents.
//!
//! Registers moss code intelligence functions into the spore Lua runtime:
//! - `moss.view(path)` - View file/symbol structure
//! - `moss.edit(path, changes)` - Structural code editing
//! - `moss.analyze.complexity(path)` - Complexity analysis
//! - `moss.analyze.health(path)` - Codebase health check
//! - `moss.analyze.security(path)` - Security analysis
//! - `moss.search(pattern)` - Text search across codebase

use mlua::{Lua, Result, Table};
use rhizome_spore_lua::Integration;

/// Moss integration for spore.
pub struct MossIntegration {
    root: std::path::PathBuf,
}

impl MossIntegration {
    /// Create a new moss integration rooted at the given path.
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Integration for MossIntegration {
    fn register(&self, lua: &Lua) -> Result<()> {
        let moss = lua.create_table()?;

        // moss.view
        register_view(&moss, lua, &self.root)?;

        // moss.edit
        register_edit(&moss, lua, &self.root)?;

        // moss.analyze.{complexity, health, security, ...}
        register_analyze(&moss, lua, &self.root)?;

        // moss.search
        register_search(&moss, lua, &self.root)?;

        lua.globals().set("moss", moss)?;
        Ok(())
    }
}

fn register_view(moss: &Table, lua: &Lua, _root: &std::path::Path) -> Result<()> {
    moss.set(
        "view",
        lua.create_function(|_, path: String| {
            // TODO: Call rhizome_moss::commands::view
            Ok(format!("view: {}", path))
        })?,
    )?;
    Ok(())
}

fn register_edit(moss: &Table, lua: &Lua, _root: &std::path::Path) -> Result<()> {
    moss.set(
        "edit",
        lua.create_function(|_, (path, changes): (String, String)| {
            // TODO: Call rhizome_moss::commands::edit
            Ok(format!("edit: {} with {}", path, changes))
        })?,
    )?;
    Ok(())
}

fn register_analyze(moss: &Table, lua: &Lua, _root: &std::path::Path) -> Result<()> {
    let analyze = lua.create_table()?;

    // moss.analyze.complexity
    analyze.set(
        "complexity",
        lua.create_function(|_, path: String| {
            // TODO: Call rhizome_moss::commands::analyze with complexity flag
            Ok(format!("analyze complexity: {}", path))
        })?,
    )?;

    // moss.analyze.health
    analyze.set(
        "health",
        lua.create_function(|_, path: String| {
            // TODO: Call rhizome_moss::commands::analyze with health flag
            Ok(format!("analyze health: {}", path))
        })?,
    )?;

    // moss.analyze.security
    analyze.set(
        "security",
        lua.create_function(|_, path: String| {
            // TODO: Call rhizome_moss::commands::analyze with security flag
            Ok(format!("analyze security: {}", path))
        })?,
    )?;

    // moss.analyze.duplicates
    analyze.set(
        "duplicates",
        lua.create_function(|_, path: String| {
            // TODO: Call rhizome_moss::commands::analyze with duplicates flag
            Ok(format!("analyze duplicates: {}", path))
        })?,
    )?;

    moss.set("analyze", analyze)?;
    Ok(())
}

fn register_search(moss: &Table, lua: &Lua, _root: &std::path::Path) -> Result<()> {
    moss.set(
        "search",
        lua.create_function(|_, pattern: String| {
            // TODO: Call rhizome_moss::text_search
            Ok(format!("search: {}", pattern))
        })?,
    )?;
    Ok(())
}
