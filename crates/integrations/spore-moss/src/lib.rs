//! rhizome-spore-moss: Moss integration for spore agents.
//!
//! Registers moss code intelligence functions into the spore Lua runtime:
//!
//! ## View & Search
//! - `moss.view(path)` - View file/symbol structure
//! - `moss.search(pattern, opts)` - Text search across codebase
//!
//! ## Analysis
//! - `moss.analyze.complexity(path)` - Cyclomatic complexity analysis
//! - `moss.analyze.length(path)` - Function length analysis
//! - `moss.analyze.health(path)` - Codebase health check
//!
//! ## Structural Editing
//! - `moss.edit.find(path, name, opts)` - Find a symbol by name
//! - `moss.edit.find_all(path, pattern)` - Find all symbols matching pattern
//! - `moss.edit.delete(path, name)` - Delete a symbol
//! - `moss.edit.replace(path, name, content)` - Replace a symbol
//! - `moss.edit.insert_before(path, name, content)` - Insert before a symbol
//! - `moss.edit.insert_after(path, name, content)` - Insert after a symbol
//! - `moss.edit.prepend_to(path, container, content)` - Prepend to class/impl body
//! - `moss.edit.append_to(path, container, content)` - Append to class/impl body
//! - `moss.edit.write(path, content)` - Write content to file

use mlua::{Lua, Result, Table};
use rhizome_spore_lua::Integration;
use std::path::{Path, PathBuf};

/// Moss integration for spore.
pub struct MossIntegration {
    root: PathBuf,
}

impl MossIntegration {
    /// Create a new moss integration rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Integration for MossIntegration {
    fn register(&self, lua: &Lua) -> Result<()> {
        let moss = lua.create_table()?;

        // moss.view
        register_view(&moss, lua, &self.root)?;

        // moss.search
        register_search(&moss, lua, &self.root)?;

        // moss.analyze.{complexity, length, health}
        register_analyze(&moss, lua, &self.root)?;

        // moss.edit.{find, delete, replace, insert_before, insert_after}
        register_edit(&moss, lua, &self.root)?;

        lua.globals().set("moss", moss)?;
        Ok(())
    }
}

/// Register moss.view(path) -> table of symbols
fn register_view(moss: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    moss.set(
        "view",
        lua.create_function(move |lua, path: String| {
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root.join(&path)
            };

            // Read file content
            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            // Extract symbols
            let extractor = rhizome_moss::extract::Extractor::new();
            let result = extractor.extract(&full_path, &content);

            // Convert to Lua table
            let symbols = lua.create_table()?;
            for (i, sym) in result.symbols.iter().enumerate() {
                symbols.set(i + 1, symbol_to_table(lua, sym)?)?;
            }

            let output = lua.create_table()?;
            output.set("file", result.file_path)?;
            output.set("symbols", symbols)?;
            Ok(output)
        })?,
    )?;
    Ok(())
}

/// Convert a Symbol to a Lua table
fn symbol_to_table(lua: &Lua, sym: &rhizome_moss_languages::Symbol) -> Result<Table> {
    let t = lua.create_table()?;
    t.set("name", sym.name.clone())?;
    t.set("kind", sym.kind.as_str())?;
    t.set("signature", sym.signature.clone())?;
    t.set("start_line", sym.start_line)?;
    t.set("end_line", sym.end_line)?;

    if let Some(doc) = &sym.docstring {
        t.set("docstring", doc.clone())?;
    }

    // Recurse for children
    if !sym.children.is_empty() {
        let children = lua.create_table()?;
        for (i, child) in sym.children.iter().enumerate() {
            children.set(i + 1, symbol_to_table(lua, child)?)?;
        }
        t.set("children", children)?;
    }

    Ok(t)
}

/// Register moss.search(pattern, opts) -> matches
fn register_search(moss: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    moss.set(
        "search",
        lua.create_function(move |lua, args: (String, Option<Table>)| {
            let (pattern, opts) = args;

            // Parse options
            let limit = opts
                .as_ref()
                .and_then(|t| t.get::<u32>("limit").ok())
                .unwrap_or(100) as usize;
            let ignore_case = opts
                .as_ref()
                .and_then(|t| t.get::<bool>("ignore_case").ok())
                .unwrap_or(false);

            // Run grep
            let result = rhizome_moss::text_search::grep(&pattern, &root, None, limit, ignore_case)
                .map_err(|e| mlua::Error::external(format!("Search failed: {}", e)))?;

            // Convert to Lua table
            let matches = lua.create_table()?;
            for (i, m) in result.matches.iter().enumerate() {
                let match_table = lua.create_table()?;
                match_table.set("file", m.file.clone())?;
                match_table.set("line", m.line)?;
                match_table.set("content", m.content.clone())?;
                if let Some(sym) = &m.symbol {
                    match_table.set("symbol", sym.clone())?;
                }
                matches.set(i + 1, match_table)?;
            }

            let output = lua.create_table()?;
            output.set("matches", matches)?;
            output.set("total", result.total_matches)?;
            output.set("files_searched", result.files_searched)?;
            Ok(output)
        })?,
    )?;
    Ok(())
}

/// Register moss.analyze.{complexity, length, health}
fn register_analyze(moss: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let analyze = lua.create_table()?;

    // moss.analyze.complexity(path) -> complexity report
    let root_complexity = root.to_path_buf();
    analyze.set(
        "complexity",
        lua.create_function(move |lua, path: String| {
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_complexity.join(&path)
            };

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            let analyzer = rhizome_moss::analyze::complexity::ComplexityAnalyzer::new();
            let report = analyzer.analyze(&full_path, &content);

            // Convert to Lua table
            let functions = lua.create_table()?;
            for (i, f) in report.functions.iter().enumerate() {
                let func = lua.create_table()?;
                func.set("name", f.name.clone())?;
                func.set("complexity", f.complexity)?;
                func.set("start_line", f.start_line)?;
                func.set("risk", f.risk_level().as_str())?;
                if let Some(parent) = &f.parent {
                    func.set("parent", parent.clone())?;
                }
                functions.set(i + 1, func)?;
            }

            let output = lua.create_table()?;
            output.set("file", report.file_path.clone())?;
            output.set("functions", functions)?;
            output.set("avg_complexity", report.avg_complexity())?;
            output.set("max_complexity", report.max_complexity())?;
            output.set("high_risk_count", report.high_risk_count())?;
            output.set("critical_risk_count", report.critical_risk_count())?;
            output.set("score", report.score())?;
            Ok(output)
        })?,
    )?;

    // moss.analyze.length(path) -> function length report
    let root_length = root.to_path_buf();
    analyze.set(
        "length",
        lua.create_function(move |lua, path: String| {
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_length.join(&path)
            };

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            let analyzer = rhizome_moss::analyze::function_length::LengthAnalyzer::new();
            let report = analyzer.analyze(&full_path, &content);

            // Convert to Lua table
            let functions = lua.create_table()?;
            for (i, f) in report.functions.iter().enumerate() {
                let func = lua.create_table()?;
                func.set("name", f.name.clone())?;
                func.set("lines", f.lines)?;
                func.set("start_line", f.start_line)?;
                func.set("end_line", f.end_line)?;
                func.set("category", f.category().as_str())?;
                if let Some(parent) = &f.parent {
                    func.set("parent", parent.clone())?;
                }
                functions.set(i + 1, func)?;
            }

            let output = lua.create_table()?;
            output.set("file", report.file_path.clone())?;
            output.set("functions", functions)?;
            output.set("avg_length", report.avg_length())?;
            output.set("max_length", report.max_length())?;
            output.set("long_count", report.long_count())?;
            output.set("too_long_count", report.too_long_count())?;
            Ok(output)
        })?,
    )?;

    // moss.analyze.health(path) -> health report for directory
    let root_health = root.to_path_buf();
    analyze.set(
        "health",
        lua.create_function(move |lua, path: Option<String>| {
            let target = path
                .map(|p| {
                    if Path::new(&p).is_absolute() {
                        PathBuf::from(p)
                    } else {
                        root_health.join(p)
                    }
                })
                .unwrap_or_else(|| root_health.clone());

            let report = rhizome_moss::health::analyze_health(&target);

            let output = lua.create_table()?;
            output.set("total_files", report.total_files)?;
            output.set("total_lines", report.total_lines)?;
            output.set("total_functions", report.total_functions)?;
            output.set("avg_complexity", report.avg_complexity)?;
            output.set("max_complexity", report.max_complexity)?;
            output.set("high_risk_functions", report.high_risk_functions)?;

            // Files by language
            let files_by_lang = lua.create_table()?;
            for (lang, count) in &report.files_by_language {
                files_by_lang.set(lang.clone(), *count)?;
            }
            output.set("files_by_language", files_by_lang)?;

            // Large files
            let large_files = lua.create_table()?;
            for (i, lf) in report.large_files.iter().enumerate() {
                let lf_table = lua.create_table()?;
                lf_table.set("path", lf.path.clone())?;
                lf_table.set("lines", lf.lines)?;
                large_files.set(i + 1, lf_table)?;
            }
            output.set("large_files", large_files)?;

            Ok(output)
        })?,
    )?;

    moss.set("analyze", analyze)?;
    Ok(())
}

/// Register moss.edit.{find, delete, replace, insert_before, insert_after}
fn register_edit(moss: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let edit = lua.create_table()?;

    // moss.edit.find(path, name, opts) -> symbol location or nil
    let root_find = root.to_path_buf();
    edit.set(
        "find",
        lua.create_function(move |lua, args: (String, String, Option<Table>)| {
            let (path, name, opts) = args;
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_find.join(&path)
            };

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            let case_insensitive = opts
                .as_ref()
                .and_then(|t| t.get::<bool>("ignore_case").ok())
                .unwrap_or(false);

            let editor = rhizome_moss::edit::Editor::new();
            match editor.find_symbol(&full_path, &content, &name, case_insensitive) {
                Some(loc) => {
                    let output = lua.create_table()?;
                    output.set("name", loc.name)?;
                    output.set("kind", loc.kind)?;
                    output.set("start_line", loc.start_line)?;
                    output.set("end_line", loc.end_line)?;
                    output.set("start_byte", loc.start_byte)?;
                    output.set("end_byte", loc.end_byte)?;
                    Ok(mlua::Value::Table(output))
                }
                None => Ok(mlua::Value::Nil),
            }
        })?,
    )?;

    // moss.edit.delete(path, name) -> modified content
    let root_delete = root.to_path_buf();
    edit.set(
        "delete",
        lua.create_function(move |_, args: (String, String)| {
            let (path, name) = args;
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_delete.join(&path)
            };

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            let editor = rhizome_moss::edit::Editor::new();
            let loc = editor
                .find_symbol(&full_path, &content, &name, false)
                .ok_or_else(|| mlua::Error::external(format!("Symbol not found: {}", name)))?;

            let result = editor.delete_symbol(&content, &loc);
            Ok(result)
        })?,
    )?;

    // moss.edit.replace(path, name, new_content) -> modified content
    let root_replace = root.to_path_buf();
    edit.set(
        "replace",
        lua.create_function(move |_, args: (String, String, String)| {
            let (path, name, new_content) = args;
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_replace.join(&path)
            };

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            let editor = rhizome_moss::edit::Editor::new();
            let loc = editor
                .find_symbol(&full_path, &content, &name, false)
                .ok_or_else(|| mlua::Error::external(format!("Symbol not found: {}", name)))?;

            let result = editor.replace_symbol(&content, &loc, &new_content);
            Ok(result)
        })?,
    )?;

    // moss.edit.insert_before(path, name, new_content) -> modified content
    let root_before = root.to_path_buf();
    edit.set(
        "insert_before",
        lua.create_function(move |_, args: (String, String, String)| {
            let (path, name, new_content) = args;
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_before.join(&path)
            };

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            let editor = rhizome_moss::edit::Editor::new();
            let loc = editor
                .find_symbol(&full_path, &content, &name, false)
                .ok_or_else(|| mlua::Error::external(format!("Symbol not found: {}", name)))?;

            let result = editor.insert_before(&content, &loc, &new_content);
            Ok(result)
        })?,
    )?;

    // moss.edit.insert_after(path, name, new_content) -> modified content
    let root_after = root.to_path_buf();
    edit.set(
        "insert_after",
        lua.create_function(move |_, args: (String, String, String)| {
            let (path, name, new_content) = args;
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_after.join(&path)
            };

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            let editor = rhizome_moss::edit::Editor::new();
            let loc = editor
                .find_symbol(&full_path, &content, &name, false)
                .ok_or_else(|| mlua::Error::external(format!("Symbol not found: {}", name)))?;

            let result = editor.insert_after(&content, &loc, &new_content);
            Ok(result)
        })?,
    )?;

    // moss.edit.write(path, content) -> writes content to file
    let root_write = root.to_path_buf();
    edit.set(
        "write",
        lua.create_function(move |_, args: (String, String)| {
            let (path, content) = args;
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_write.join(&path)
            };

            std::fs::write(&full_path, &content)
                .map_err(|e| mlua::Error::external(format!("Failed to write {}: {}", path, e)))?;
            Ok(true)
        })?,
    )?;

    // moss.edit.find_all(path, pattern) -> array of symbol locations
    let root_find_all = root.to_path_buf();
    edit.set(
        "find_all",
        lua.create_function(move |lua, args: (String, String)| {
            let (path, pattern) = args;
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_find_all.join(&path)
            };

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            let editor = rhizome_moss::edit::Editor::new();
            let locations = editor.find_symbols_matching(&full_path, &content, &pattern);

            let output = lua.create_table()?;
            for (i, loc) in locations.iter().enumerate() {
                let loc_table = lua.create_table()?;
                loc_table.set("name", loc.name.clone())?;
                loc_table.set("kind", loc.kind.clone())?;
                loc_table.set("start_line", loc.start_line)?;
                loc_table.set("end_line", loc.end_line)?;
                loc_table.set("start_byte", loc.start_byte)?;
                loc_table.set("end_byte", loc.end_byte)?;
                output.set(i + 1, loc_table)?;
            }
            Ok(output)
        })?,
    )?;

    // moss.edit.prepend_to(path, container, content) -> modified content
    let root_prepend = root.to_path_buf();
    edit.set(
        "prepend_to",
        lua.create_function(move |_, args: (String, String, String)| {
            let (path, container, new_content) = args;
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_prepend.join(&path)
            };

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            let editor = rhizome_moss::edit::Editor::new();
            let body = editor
                .find_container_body(&full_path, &content, &container)
                .ok_or_else(|| {
                    mlua::Error::external(format!("Container not found: {}", container))
                })?;

            let result = editor.prepend_to_container(&content, &body, &new_content);
            Ok(result)
        })?,
    )?;

    // moss.edit.append_to(path, container, content) -> modified content
    let root_append = root.to_path_buf();
    edit.set(
        "append_to",
        lua.create_function(move |_, args: (String, String, String)| {
            let (path, container, new_content) = args;
            let full_path = if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                root_append.join(&path)
            };

            let content = std::fs::read_to_string(&full_path)
                .map_err(|e| mlua::Error::external(format!("Failed to read {}: {}", path, e)))?;

            let editor = rhizome_moss::edit::Editor::new();
            let body = editor
                .find_container_body(&full_path, &content, &container)
                .ok_or_else(|| {
                    mlua::Error::external(format!("Container not found: {}", container))
                })?;

            let result = editor.append_to_container(&content, &body, &new_content);
            Ok(result)
        })?,
    )?;

    moss.set("edit", edit)?;
    Ok(())
}
