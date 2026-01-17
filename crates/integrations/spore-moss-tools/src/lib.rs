//! rhizome-spore-moss-tools: Moss tools integration for spore.
//!
//! Registers external tool execution functions into the spore Lua runtime:
//!
//! ## Tool Registry
//! - `tools.list(opts?)` - List all tool names (optionally filter by category)
//! - `tools.is_available(name)` - Check if tool binary exists
//! - `tools.detect(root?)` - Detect relevant tools for project
//! - `tools.info(name)` - Get tool info (category, extensions, etc.)
//! - `tools.run(name, paths?, opts?)` - Run tool, return diagnostics
//! - `tools.fix(name, paths?, opts?)` - Run tool in fix mode
//!
//! ## Test Runners
//! - `tools.test.list()` - List all runner names
//! - `tools.test.is_available(name)` - Check if runner available
//! - `tools.test.detect(root?)` - Detect best runner for project
//! - `tools.test.run(name?, args?, opts?)` - Run tests, return result

use mlua::{Lua, Result, Table, Value};
use rhizome_moss_tools::{
    ToolCategory, ToolResult, default_registry, get_tool,
    test_runners::{self, TestRunner},
};
use rhizome_spore_lua::Integration;
use std::path::{Path, PathBuf};

/// Moss tools integration for spore.
pub struct MossToolsIntegration {
    root: PathBuf,
}

impl MossToolsIntegration {
    /// Create a new tools integration rooted at the given path.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl Integration for MossToolsIntegration {
    fn register(&self, lua: &Lua) -> Result<()> {
        let tools = lua.create_table()?;

        // Tool registry functions
        register_tool_list(&tools, lua)?;
        register_tool_is_available(&tools, lua)?;
        register_tool_detect(&tools, lua, &self.root)?;
        register_tool_info(&tools, lua)?;
        register_tool_run(&tools, lua, &self.root)?;
        register_tool_fix(&tools, lua, &self.root)?;

        // Test runner functions (tools.test.*)
        register_test_runners(&tools, lua, &self.root)?;

        lua.globals().set("tools", tools)?;
        Ok(())
    }
}

/// Register tools.list(opts?) -> array of tool names
fn register_tool_list(tools: &Table, lua: &Lua) -> Result<()> {
    tools.set(
        "list",
        lua.create_function(|lua, opts: Option<Table>| {
            let registry = default_registry();
            let category_filter = opts
                .as_ref()
                .and_then(|t| t.get::<String>("category").ok())
                .and_then(|c| match c.as_str() {
                    "linter" => Some(ToolCategory::Linter),
                    "formatter" => Some(ToolCategory::Formatter),
                    "type-checker" | "type_checker" => Some(ToolCategory::TypeChecker),
                    _ => None,
                });

            let result = lua.create_table()?;
            let mut idx = 1;
            for tool in registry.tools() {
                if let Some(cat) = category_filter
                    && tool.info().category != cat
                {
                    continue;
                }
                result.set(idx, tool.info().name)?;
                idx += 1;
            }
            Ok(result)
        })?,
    )?;
    Ok(())
}

/// Register tools.is_available(name) -> bool
fn register_tool_is_available(tools: &Table, lua: &Lua) -> Result<()> {
    tools.set(
        "is_available",
        lua.create_function(|_, name: String| {
            if let Some(tool) = get_tool(&name) {
                Ok(tool.is_available())
            } else {
                Ok(false)
            }
        })?,
    )?;
    Ok(())
}

/// Register tools.detect(root?) -> array of {name, confidence}
fn register_tool_detect(tools: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    tools.set(
        "detect",
        lua.create_function(move |lua, path: Option<String>| {
            let target = path
                .map(|p| {
                    if Path::new(&p).is_absolute() {
                        PathBuf::from(p)
                    } else {
                        root.join(p)
                    }
                })
                .unwrap_or_else(|| root.clone());

            let registry = default_registry();
            let result = lua.create_table()?;
            let mut idx = 1;

            for tool in registry.tools() {
                if !tool.is_available() {
                    continue;
                }
                let confidence = tool.detect(&target);
                if confidence > 0.0 {
                    let entry = lua.create_table()?;
                    entry.set("name", tool.info().name)?;
                    entry.set("confidence", confidence)?;
                    result.set(idx, entry)?;
                    idx += 1;
                }
            }
            Ok(result)
        })?,
    )?;
    Ok(())
}

/// Register tools.info(name) -> tool info table or nil
fn register_tool_info(tools: &Table, lua: &Lua) -> Result<()> {
    tools.set(
        "info",
        lua.create_function(|lua, name: String| {
            if let Some(tool) = get_tool(&name) {
                let info = tool.info();
                let t = lua.create_table()?;
                t.set("name", info.name)?;
                t.set("category", info.category.as_str())?;
                t.set("website", info.website)?;

                let extensions = lua.create_table()?;
                for (i, ext) in info.extensions.iter().enumerate() {
                    extensions.set(i + 1, *ext)?;
                }
                t.set("extensions", extensions)?;

                Ok(Value::Table(t))
            } else {
                Ok(Value::Nil)
            }
        })?,
    )?;
    Ok(())
}

/// Register tools.run(name, paths?, opts?) -> result table
fn register_tool_run(tools: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    tools.set(
        "run",
        lua.create_function(
            move |lua, args: (String, Option<Vec<String>>, Option<Table>)| {
                let (name, paths, _opts) = args;

                let tool = get_tool(&name)
                    .ok_or_else(|| mlua::Error::external(format!("Tool not found: {}", name)))?;

                if !tool.is_available() {
                    return Err(mlua::Error::external(format!(
                        "Tool not available: {}",
                        name
                    )));
                }

                let path_bufs: Vec<PathBuf> = paths
                    .unwrap_or_else(|| vec![".".to_string()])
                    .into_iter()
                    .map(|p| {
                        if Path::new(&p).is_absolute() {
                            PathBuf::from(p)
                        } else {
                            root.join(p)
                        }
                    })
                    .collect();

                let path_refs: Vec<&Path> = path_bufs.iter().map(|p| p.as_path()).collect();

                let result = tool
                    .run(&path_refs, &root)
                    .map_err(|e| mlua::Error::external(format!("Tool execution failed: {}", e)))?;

                tool_result_to_lua(lua, &result)
            },
        )?,
    )?;
    Ok(())
}

/// Register tools.fix(name, paths?, opts?) -> result table
fn register_tool_fix(tools: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let root = root.to_path_buf();
    tools.set(
        "fix",
        lua.create_function(
            move |lua, args: (String, Option<Vec<String>>, Option<Table>)| {
                let (name, paths, _opts) = args;

                let tool = get_tool(&name)
                    .ok_or_else(|| mlua::Error::external(format!("Tool not found: {}", name)))?;

                if !tool.is_available() {
                    return Err(mlua::Error::external(format!(
                        "Tool not available: {}",
                        name
                    )));
                }

                if !tool.can_fix() {
                    return Err(mlua::Error::external(format!(
                        "Tool does not support fix mode: {}",
                        name
                    )));
                }

                let path_bufs: Vec<PathBuf> = paths
                    .unwrap_or_else(|| vec![".".to_string()])
                    .into_iter()
                    .map(|p| {
                        if Path::new(&p).is_absolute() {
                            PathBuf::from(p)
                        } else {
                            root.join(p)
                        }
                    })
                    .collect();

                let path_refs: Vec<&Path> = path_bufs.iter().map(|p| p.as_path()).collect();

                let result = tool
                    .fix(&path_refs, &root)
                    .map_err(|e| mlua::Error::external(format!("Tool fix failed: {}", e)))?;

                tool_result_to_lua(lua, &result)
            },
        )?,
    )?;
    Ok(())
}

/// Convert ToolResult to Lua table
fn tool_result_to_lua(lua: &Lua, result: &ToolResult) -> Result<Table> {
    let t = lua.create_table()?;
    t.set("tool", result.tool.clone())?;
    t.set("success", result.success)?;

    if let Some(err) = &result.error {
        t.set("error", err.clone())?;
    }

    let diagnostics = lua.create_table()?;
    for (i, diag) in result.diagnostics.iter().enumerate() {
        let d = lua.create_table()?;
        d.set("severity", diag.severity.as_str())?;
        d.set("message", diag.message.clone())?;
        d.set("file", diag.location.file.to_string_lossy().to_string())?;
        d.set("line", diag.location.line)?;
        d.set("column", diag.location.column)?;
        d.set("rule", diag.rule_id.clone())?;
        if let Some(fix) = &diag.fix {
            let f = lua.create_table()?;
            f.set("description", fix.description.clone())?;
            f.set("replacement", fix.replacement.clone())?;
            d.set("fix", f)?;
        }
        diagnostics.set(i + 1, d)?;
    }
    t.set("diagnostics", diagnostics)?;

    Ok(t)
}

/// Register tools.test.* functions
fn register_test_runners(tools: &Table, lua: &Lua, root: &Path) -> Result<()> {
    let test = lua.create_table()?;

    // tools.test.list() -> array of runner names
    test.set(
        "list",
        lua.create_function(|lua, ()| {
            let runners = test_runners::list_runners();
            let result = lua.create_table()?;
            for (i, name) in runners.iter().enumerate() {
                result.set(i + 1, *name)?;
            }
            Ok(result)
        })?,
    )?;

    // tools.test.is_available(name) -> bool
    test.set(
        "is_available",
        lua.create_function(|_, name: String| {
            if let Some(runner) = test_runners::get_runner(&name) {
                Ok(runner.is_available())
            } else {
                Ok(false)
            }
        })?,
    )?;

    // tools.test.detect(root?) -> runner name or nil
    let root_detect = root.to_path_buf();
    test.set(
        "detect",
        lua.create_function(move |_, path: Option<String>| {
            let target = path
                .map(|p| {
                    if Path::new(&p).is_absolute() {
                        PathBuf::from(p)
                    } else {
                        root_detect.join(p)
                    }
                })
                .unwrap_or_else(|| root_detect.clone());

            Ok(test_runners::detect_test_runner(&target).map(|r| r.info().name))
        })?,
    )?;

    // tools.test.run(name?, args?, opts?) -> result table
    let root_run = root.to_path_buf();
    test.set(
        "run",
        lua.create_function(
            move |lua, args: (Option<String>, Option<Vec<String>>, Option<Table>)| {
                let (name, extra_args, _opts) = args;

                let runner: &dyn TestRunner = if let Some(name) = name {
                    test_runners::get_runner(&name).ok_or_else(|| {
                        mlua::Error::external(format!("Test runner not found: {}", name))
                    })?
                } else {
                    test_runners::detect_test_runner(&root_run)
                        .ok_or_else(|| mlua::Error::external("No suitable test runner detected"))?
                };

                if !runner.is_available() {
                    return Err(mlua::Error::external(format!(
                        "Test runner not available: {}",
                        runner.info().name
                    )));
                }

                let args_vec: Vec<String> = extra_args.unwrap_or_default();
                let args_refs: Vec<&str> = args_vec.iter().map(|s| s.as_str()).collect();

                let start = std::time::Instant::now();
                let result = runner
                    .run(&root_run, &args_refs)
                    .map_err(|e| mlua::Error::external(format!("Test execution failed: {}", e)))?;
                let duration_ms = start.elapsed().as_millis() as u64;

                let t = lua.create_table()?;
                t.set("runner", result.runner.clone())?;
                t.set("success", result.success())?;
                t.set("duration_ms", duration_ms)?;

                Ok(t)
            },
        )?,
    )?;

    tools.set("test", test)?;
    Ok(())
}
