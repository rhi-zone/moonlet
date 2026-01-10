//! rhizome-spore-moss: Moss integration for spore agents.
//!
//! Registers moss code intelligence functions into the spore Lua runtime:
//! - `moss.view(path)` - View file/symbol structure
//! - `moss.search(pattern, opts)` - Text search across codebase
//! - `moss.analyze.complexity(path)` - Complexity analysis
//! - `moss.analyze.health(path)` - Codebase health check

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

        // moss.analyze.{complexity, health}
        register_analyze(&moss, lua, &self.root)?;

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
            let limit = opts.as_ref()
                .and_then(|t| t.get::<u32>("limit").ok())
                .unwrap_or(100) as usize;
            let ignore_case = opts.as_ref()
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

/// Register moss.analyze.{complexity, health}
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
