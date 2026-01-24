//! Development tools plugin for spore.
//!
//! Provides capability-based access to linters, formatters, type checkers, and test runners.
//!
//! ## Module Functions (no capability needed)
//! - `tools.list(opts?)` - List all tool names
//! - `tools.is_available(name)` - Check if tool binary exists
//! - `tools.info(name)` - Get tool info
//! - `tools.test_list()` - List test runner names
//! - `tools.test_is_available(name)` - Check if test runner available
//!
//! ## Capability Constructor
//! - `tools.capability({ root = "..." })` - Create tools capability for project
//!
//! ## Capability Methods
//! - `cap:detect()` - Detect relevant tools for project
//! - `cap:run(name, paths?, opts?)` - Run tool
//! - `cap:fix(name, paths?, opts?)` - Run tool in fix mode
//! - `cap:test_detect()` - Detect best test runner
//! - `cap:test_run(name?, args?, opts?)` - Run tests (blocking)
//! - `cap:test_start(name?, args?, opts?)` - Run tests (async, returns Handle)

#![allow(non_snake_case)]

use mlua::ffi::{self, lua_State};
use moonlet_lua::handle::{self, Handle, HandleItem, HandleResult, Stream};
use normalize_tools::{
    ToolCategory, ToolResult, default_registry, get_tool,
    test_runners::{self, TestRunner},
};
use std::ffi::{CStr, CString, c_char, c_int};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;

/// Plugin ABI version.
const ABI_VERSION: u32 = 1;

/// Metatable name for ToolsCapability userdata.
const TOOLS_CAP_METATABLE: &[u8] = b"spore.tools.Capability\0";

/// Plugin info for version checking.
#[repr(C)]
pub struct PluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub abi_version: u32,
}

// ============================================================================
// Capability
// ============================================================================

/// Tools capability - provides access to run tools on a project root.
#[derive(Debug, Clone)]
pub struct ToolsCapability {
    root: PathBuf,
}

impl ToolsCapability {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

// ============================================================================
// Plugin exports
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn moonlet_plugin_info() -> PluginInfo {
    PluginInfo {
        name: c"tools".as_ptr(),
        version: c"0.1.0".as_ptr(),
        abi_version: ABI_VERSION,
    }
}

/// Lua module entry point.
///
/// # Safety
/// Must be called from Lua with a valid lua_State pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_moonlet_tools(L: *mut lua_State) -> c_int {
    unsafe {
        // Register Handle metatable (from spore-lua)
        handle::register_handle_metatable(L);

        // Register capability metatable
        register_capability_metatable(L);

        // Create module table
        ffi::lua_createtable(L, 0, 7);

        // Module functions (no capability needed)
        ffi::lua_pushcclosure(L, tools_list, 0);
        ffi::lua_setfield(L, -2, c"list".as_ptr());

        ffi::lua_pushcclosure(L, tools_is_available, 0);
        ffi::lua_setfield(L, -2, c"is_available".as_ptr());

        ffi::lua_pushcclosure(L, tools_info, 0);
        ffi::lua_setfield(L, -2, c"info".as_ptr());

        ffi::lua_pushcclosure(L, tools_test_list, 0);
        ffi::lua_setfield(L, -2, c"test_list".as_ptr());

        ffi::lua_pushcclosure(L, tools_test_is_available, 0);
        ffi::lua_setfield(L, -2, c"test_is_available".as_ptr());

        // Capability constructor
        ffi::lua_pushcclosure(L, tools_capability, 0);
        ffi::lua_setfield(L, -2, c"capability".as_ptr());

        1
    }
}

// ============================================================================
// Capability metatable
// ============================================================================

unsafe fn register_capability_metatable(L: *mut lua_State) {
    unsafe {
        if ffi::luaL_newmetatable(L, TOOLS_CAP_METATABLE.as_ptr() as *const c_char) != 0 {
            ffi::lua_createtable(L, 0, 7);

            ffi::lua_pushcclosure(L, cap_detect, 0);
            ffi::lua_setfield(L, -2, c"detect".as_ptr());

            ffi::lua_pushcclosure(L, cap_run, 0);
            ffi::lua_setfield(L, -2, c"run".as_ptr());

            ffi::lua_pushcclosure(L, cap_fix, 0);
            ffi::lua_setfield(L, -2, c"fix".as_ptr());

            ffi::lua_pushcclosure(L, cap_test_detect, 0);
            ffi::lua_setfield(L, -2, c"test_detect".as_ptr());

            ffi::lua_pushcclosure(L, cap_test_run, 0);
            ffi::lua_setfield(L, -2, c"test_run".as_ptr());

            ffi::lua_pushcclosure(L, cap_test_start, 0);
            ffi::lua_setfield(L, -2, c"test_start".as_ptr());

            ffi::lua_pushcclosure(L, cap_attenuate, 0);
            ffi::lua_setfield(L, -2, c"attenuate".as_ptr());

            ffi::lua_setfield(L, -2, c"__index".as_ptr());

            ffi::lua_pushcclosure(L, cap_gc, 0);
            ffi::lua_setfield(L, -2, c"__gc".as_ptr());

            ffi::lua_pushcclosure(L, cap_tostring, 0);
            ffi::lua_setfield(L, -2, c"__tostring".as_ptr());
        }
        ffi::lua_pop(L, 1);
    }
}

// ============================================================================
// Module functions (no capability needed)
// ============================================================================

/// tools.list(opts?) -> array of tool names
unsafe extern "C-unwind" fn tools_list(L: *mut lua_State) -> c_int {
    unsafe {
        let registry = default_registry();

        // Check for category filter
        let category_filter = if ffi::lua_type(L, 1) == ffi::LUA_TTABLE {
            ffi::lua_getfield(L, 1, c"category".as_ptr());
            let cat = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                let ptr = ffi::lua_tostring(L, -1);
                let s = CStr::from_ptr(ptr).to_string_lossy();
                match s.as_ref() {
                    "linter" => Some(ToolCategory::Linter),
                    "formatter" => Some(ToolCategory::Formatter),
                    "type-checker" | "type_checker" => Some(ToolCategory::TypeChecker),
                    _ => None,
                }
            } else {
                None
            };
            ffi::lua_pop(L, 1);
            cat
        } else {
            None
        };

        ffi::lua_createtable(L, 0, 0);
        let mut idx = 1;

        for tool in registry.tools() {
            if let Some(cat) = category_filter
                && tool.info().category != cat
            {
                continue;
            }
            let c_name = CString::new(tool.info().name).unwrap();
            ffi::lua_pushstring(L, c_name.as_ptr());
            ffi::lua_rawseti(L, -2, idx);
            idx += 1;
        }

        1
    }
}

/// tools.is_available(name) -> bool
unsafe extern "C-unwind" fn tools_is_available(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "is_available requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 1);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        let available = get_tool(&name).map(|t| t.is_available()).unwrap_or(false);
        ffi::lua_pushboolean(L, available as c_int);
        1
    }
}

/// tools.info(name) -> info table or nil
unsafe extern "C-unwind" fn tools_info(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "info requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 1);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        match get_tool(&name) {
            Some(tool) => {
                let info = tool.info();
                ffi::lua_createtable(L, 0, 4);

                let c_name = CString::new(info.name).unwrap();
                ffi::lua_pushstring(L, c_name.as_ptr());
                ffi::lua_setfield(L, -2, c"name".as_ptr());

                let c_cat = CString::new(info.category.as_str()).unwrap();
                ffi::lua_pushstring(L, c_cat.as_ptr());
                ffi::lua_setfield(L, -2, c"category".as_ptr());

                let c_website = CString::new(info.website).unwrap();
                ffi::lua_pushstring(L, c_website.as_ptr());
                ffi::lua_setfield(L, -2, c"website".as_ptr());

                ffi::lua_createtable(L, info.extensions.len() as c_int, 0);
                for (i, ext) in info.extensions.iter().enumerate() {
                    let c_ext = CString::new(*ext).unwrap();
                    ffi::lua_pushstring(L, c_ext.as_ptr());
                    ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
                }
                ffi::lua_setfield(L, -2, c"extensions".as_ptr());

                1
            }
            None => {
                ffi::lua_pushnil(L);
                1
            }
        }
    }
}

/// tools.test_list() -> array of test runner names
unsafe extern "C-unwind" fn tools_test_list(L: *mut lua_State) -> c_int {
    unsafe {
        let runners = test_runners::list_runners();
        ffi::lua_createtable(L, runners.len() as c_int, 0);

        for (i, name) in runners.iter().enumerate() {
            let c_name = CString::new(*name).unwrap();
            ffi::lua_pushstring(L, c_name.as_ptr());
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }

        1
    }
}

/// tools.test_is_available(name) -> bool
unsafe extern "C-unwind" fn tools_test_is_available(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "test_is_available requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 1);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        let available = test_runners::get_runner(&name)
            .map(|r| r.is_available())
            .unwrap_or(false);
        ffi::lua_pushboolean(L, available as c_int);
        1
    }
}

// ============================================================================
// Capability constructor
// ============================================================================

/// tools.capability({ root = "..." }) -> ToolsCapability
unsafe extern "C-unwind" fn tools_capability(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TTABLE {
            return push_error(L, "capability expects a table argument");
        }

        // Get root path
        ffi::lua_getfield(L, 1, c"root".as_ptr());
        if ffi::lua_type(L, -1) != ffi::LUA_TSTRING {
            return push_error(L, "capability requires 'root' string");
        }
        let root_ptr = ffi::lua_tostring(L, -1);
        let root = CStr::from_ptr(root_ptr).to_string_lossy().into_owned();
        ffi::lua_pop(L, 1);

        create_capability_userdata(L, ToolsCapability::new(PathBuf::from(root)))
    }
}

unsafe fn create_capability_userdata(L: *mut lua_State, cap: ToolsCapability) -> c_int {
    unsafe {
        let boxed = Box::new(cap);
        let ud = ffi::lua_newuserdata(L, std::mem::size_of::<*mut ToolsCapability>());
        let ud_ptr = ud as *mut *mut ToolsCapability;
        *ud_ptr = Box::into_raw(boxed);

        ffi::luaL_newmetatable(L, TOOLS_CAP_METATABLE.as_ptr() as *const c_char);
        ffi::lua_setmetatable(L, -2);

        1
    }
}

unsafe fn get_capability(L: *mut lua_State, idx: c_int) -> Option<&'static ToolsCapability> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, idx, TOOLS_CAP_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return None;
        }
        let cap_ptr = *(ud as *const *mut ToolsCapability);
        if cap_ptr.is_null() {
            return None;
        }
        Some(&*cap_ptr)
    }
}

// ============================================================================
// Capability methods
// ============================================================================

/// cap:detect() -> array of {name, confidence}
unsafe extern "C-unwind" fn cap_detect(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        let registry = default_registry();
        ffi::lua_createtable(L, 0, 0);
        let mut idx = 1;

        for tool in registry.tools() {
            if !tool.is_available() {
                continue;
            }
            let confidence = tool.detect(&cap.root);
            if confidence > 0.0 {
                ffi::lua_createtable(L, 0, 2);

                let c_name = CString::new(tool.info().name).unwrap();
                ffi::lua_pushstring(L, c_name.as_ptr());
                ffi::lua_setfield(L, -2, c"name".as_ptr());

                ffi::lua_pushnumber(L, confidence as f64);
                ffi::lua_setfield(L, -2, c"confidence".as_ptr());

                ffi::lua_rawseti(L, -2, idx);
                idx += 1;
            }
        }

        1
    }
}

/// cap:run(name, paths?, opts?) -> result table
unsafe extern "C-unwind" fn cap_run(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "run requires tool name");
        }
        let name_ptr = ffi::lua_tostring(L, 2);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        let tool = match get_tool(&name) {
            Some(t) => t,
            None => return push_error(L, &format!("Tool not found: {}", name)),
        };

        if !tool.is_available() {
            return push_error(L, &format!("Tool not available: {}", name));
        }

        // Get paths (optional)
        let paths = get_paths_arg(L, 3, &cap.root);
        let path_refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();

        match tool.run(&path_refs, &cap.root) {
            Ok(result) => push_tool_result(L, &result),
            Err(e) => push_error(L, &format!("Tool execution failed: {}", e)),
        }
    }
}

/// cap:fix(name, paths?, opts?) -> result table
unsafe extern "C-unwind" fn cap_fix(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "fix requires tool name");
        }
        let name_ptr = ffi::lua_tostring(L, 2);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        let tool = match get_tool(&name) {
            Some(t) => t,
            None => return push_error(L, &format!("Tool not found: {}", name)),
        };

        if !tool.is_available() {
            return push_error(L, &format!("Tool not available: {}", name));
        }

        if !tool.can_fix() {
            return push_error(L, &format!("Tool does not support fix mode: {}", name));
        }

        let paths = get_paths_arg(L, 3, &cap.root);
        let path_refs: Vec<&Path> = paths.iter().map(|p| p.as_path()).collect();

        match tool.fix(&path_refs, &cap.root) {
            Ok(result) => push_tool_result(L, &result),
            Err(e) => push_error(L, &format!("Tool fix failed: {}", e)),
        }
    }
}

/// cap:test_detect() -> runner name or nil
unsafe extern "C-unwind" fn cap_test_detect(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        match test_runners::detect_test_runner(&cap.root) {
            Some(runner) => {
                let c_name = CString::new(runner.info().name).unwrap();
                ffi::lua_pushstring(L, c_name.as_ptr());
                1
            }
            None => {
                ffi::lua_pushnil(L);
                1
            }
        }
    }
}

/// cap:test_run(name?, args?, opts?) -> result table
unsafe extern "C-unwind" fn cap_test_run(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        // Get runner name (optional)
        let runner: &dyn TestRunner = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let name_ptr = ffi::lua_tostring(L, 2);
            let name = CStr::from_ptr(name_ptr).to_string_lossy();
            match test_runners::get_runner(&name) {
                Some(r) => r,
                None => return push_error(L, &format!("Test runner not found: {}", name)),
            }
        } else {
            match test_runners::detect_test_runner(&cap.root) {
                Some(r) => r,
                None => return push_error(L, "No suitable test runner detected"),
            }
        };

        if !runner.is_available() {
            return push_error(
                L,
                &format!("Test runner not available: {}", runner.info().name),
            );
        }

        // Get extra args (optional)
        let args = get_string_array_arg(L, 3);
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let start = std::time::Instant::now();
        match runner.run(&cap.root, &arg_refs) {
            Ok(result) => {
                let duration_ms = start.elapsed().as_millis() as u64;

                ffi::lua_createtable(L, 0, 3);

                let c_name = CString::new(result.runner.as_str()).unwrap();
                ffi::lua_pushstring(L, c_name.as_ptr());
                ffi::lua_setfield(L, -2, c"runner".as_ptr());

                ffi::lua_pushboolean(L, result.success() as c_int);
                ffi::lua_setfield(L, -2, c"success".as_ptr());

                ffi::lua_pushinteger(L, duration_ms as ffi::lua_Integer);
                ffi::lua_setfield(L, -2, c"duration_ms".as_ptr());

                1
            }
            Err(e) => push_error(L, &format!("Test execution failed: {}", e)),
        }
    }
}

/// cap:test_start(name?, args?, opts?) -> Handle
/// Starts test runner asynchronously, returning a Handle for streaming output.
unsafe extern "C-unwind" fn cap_test_start(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        // Get runner name (optional)
        let runner: &dyn TestRunner = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let name_ptr = ffi::lua_tostring(L, 2);
            let name = CStr::from_ptr(name_ptr).to_string_lossy();
            match test_runners::get_runner(&name) {
                Some(r) => r,
                None => return push_error(L, &format!("Test runner not found: {}", name)),
            }
        } else {
            match test_runners::detect_test_runner(&cap.root) {
                Some(r) => r,
                None => return push_error(L, "No suitable test runner detected"),
            }
        };

        if !runner.is_available() {
            return push_error(
                L,
                &format!("Test runner not available: {}", runner.info().name),
            );
        }

        // Get extra args (optional)
        let args = get_string_array_arg(L, 3);

        // Build the command based on runner name
        let runner_name = runner.info().name;
        let (cmd_name, base_args) = get_runner_command(runner_name);
        let mut cmd = Command::new(&cmd_name);
        cmd.current_dir(&cap.root);
        cmd.args(&base_args);
        cmd.args(&args);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        // Spawn the process
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => return push_error(L, &format!("Failed to spawn test runner: {}", e)),
        };

        // Create Handle for async streaming
        let handle = spawn_test_process(runner_name.to_string(), child);
        handle::push_handle(L, handle)
    }
}

/// Map runner name to command and base args.
/// This mirrors the command building in normalize-tools test runners.
fn get_runner_command(name: &str) -> (String, Vec<&'static str>) {
    match name {
        "cargo" => ("cargo".to_string(), vec!["test"]),
        "pytest" => ("pytest".to_string(), vec![]),
        "npm" => ("npm".to_string(), vec!["test"]),
        "jest" => ("npx".to_string(), vec!["jest"]),
        "mocha" => ("npx".to_string(), vec!["mocha"]),
        "go" => ("go".to_string(), vec!["test", "./..."]),
        "maven" | "mvn" => ("mvn".to_string(), vec!["test"]),
        "gradle" => ("gradle".to_string(), vec!["test"]),
        "rspec" => ("bundle".to_string(), vec!["exec", "rspec"]),
        "phpunit" => ("vendor/bin/phpunit".to_string(), vec![]),
        "dotnet" => ("dotnet".to_string(), vec!["test"]),
        // Default: try running the name directly
        _ => (name.to_string(), vec![]),
    }
}

/// Spawn a test process and return a Handle for streaming output.
fn spawn_test_process(name: String, mut child: std::process::Child) -> Handle {
    use std::time::Duration;

    let (tx, rx) = channel();
    let (kill_tx, kill_rx) = channel::<()>();

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // Spawn thread to read stdout
    let tx_stdout = tx.clone();
    if let Some(stdout) = stdout {
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx_stdout.send(HandleItem {
                    stream: Stream::Stdout,
                    content: line,
                });
            }
        });
    }

    // Spawn thread to read stderr
    let tx_stderr = tx;
    if let Some(stderr) = stderr {
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                let _ = tx_stderr.send(HandleItem {
                    stream: Stream::Stderr,
                    content: line,
                });
            }
        });
    }

    // Spawn thread to wait for completion and handle kill
    let join_handle = std::thread::spawn(move || {
        loop {
            // Check for kill signal
            if kill_rx.try_recv().is_ok() {
                let _ = child.kill();
                return HandleResult {
                    success: false,
                    exit_code: None,
                    data: Some("killed".to_string()),
                };
            }

            // Check if child has exited
            match child.try_wait() {
                Ok(Some(status)) => {
                    return HandleResult {
                        success: status.success(),
                        exit_code: status.code(),
                        data: None,
                    };
                }
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    return HandleResult {
                        success: false,
                        exit_code: None,
                        data: Some(e.to_string()),
                    };
                }
            }
        }
    });

    Handle::new(name, rx, Some(join_handle), Some(kill_tx))
}

/// cap:attenuate({ root = "subdir" }) -> new capability
unsafe extern "C-unwind" fn cap_attenuate(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(cap) = get_capability(L, 1) else {
            return push_error(L, "invalid capability");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TTABLE {
            return push_error(L, "attenuate expects a table argument");
        }

        // Get new root (relative to current)
        ffi::lua_getfield(L, 2, c"root".as_ptr());
        let new_root = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, -1);
            let rel = CStr::from_ptr(ptr).to_string_lossy();
            let full = cap.root.join(rel.as_ref());

            // Ensure it doesn't escape original root
            let canonical = if full.exists() {
                full.canonicalize().unwrap_or(full)
            } else {
                full
            };
            let root_canonical = if cap.root.exists() {
                cap.root.canonicalize().unwrap_or(cap.root.clone())
            } else {
                cap.root.clone()
            };

            if !canonical.starts_with(&root_canonical) {
                return push_error(L, "path escapes capability root");
            }
            canonical
        } else {
            cap.root.clone()
        };
        ffi::lua_pop(L, 1);

        create_capability_userdata(L, ToolsCapability::new(new_root))
    }
}

unsafe extern "C-unwind" fn cap_gc(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1);
        if !ud.is_null() {
            let cap_ptr = *(ud as *mut *mut ToolsCapability);
            if !cap_ptr.is_null() {
                drop(Box::from_raw(cap_ptr));
            }
        }
        0
    }
}

unsafe extern "C-unwind" fn cap_tostring(L: *mut lua_State) -> c_int {
    unsafe {
        if let Some(cap) = get_capability(L, 1) {
            let s = format!("ToolsCapability(root={:?})", cap.root);
            let c_s = CString::new(s).unwrap();
            ffi::lua_pushstring(L, c_s.as_ptr());
        } else {
            ffi::lua_pushstring(L, c"ToolsCapability(invalid)".as_ptr());
        }
        1
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Get paths array from argument, defaulting to ["."]
unsafe fn get_paths_arg(L: *mut lua_State, idx: c_int, root: &Path) -> Vec<PathBuf> {
    unsafe {
        if ffi::lua_type(L, idx) == ffi::LUA_TTABLE {
            let mut paths = Vec::new();
            let len = ffi::lua_rawlen(L, idx);
            for i in 1..=len {
                ffi::lua_rawgeti(L, idx, i as ffi::lua_Integer);
                if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                    let ptr = ffi::lua_tostring(L, -1);
                    let s = CStr::from_ptr(ptr).to_string_lossy();
                    let p = Path::new(s.as_ref());
                    if p.is_absolute() {
                        paths.push(p.to_path_buf());
                    } else {
                        paths.push(root.join(p));
                    }
                }
                ffi::lua_pop(L, 1);
            }
            if paths.is_empty() {
                paths.push(root.to_path_buf());
            }
            paths
        } else {
            vec![root.to_path_buf()]
        }
    }
}

/// Get string array from argument
unsafe fn get_string_array_arg(L: *mut lua_State, idx: c_int) -> Vec<String> {
    unsafe {
        if ffi::lua_type(L, idx) == ffi::LUA_TTABLE {
            let mut strings = Vec::new();
            let len = ffi::lua_rawlen(L, idx);
            for i in 1..=len {
                ffi::lua_rawgeti(L, idx, i as ffi::lua_Integer);
                if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                    let ptr = ffi::lua_tostring(L, -1);
                    let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
                    strings.push(s);
                }
                ffi::lua_pop(L, 1);
            }
            strings
        } else {
            Vec::new()
        }
    }
}

/// Push a ToolResult as a Lua table.
unsafe fn push_tool_result(L: *mut lua_State, result: &ToolResult) -> c_int {
    unsafe {
        ffi::lua_createtable(L, 0, 4);

        let c_tool = CString::new(result.tool.as_str()).unwrap();
        ffi::lua_pushstring(L, c_tool.as_ptr());
        ffi::lua_setfield(L, -2, c"tool".as_ptr());

        ffi::lua_pushboolean(L, result.success as c_int);
        ffi::lua_setfield(L, -2, c"success".as_ptr());

        if let Some(err) = &result.error {
            let c_err = CString::new(err.as_str()).unwrap();
            ffi::lua_pushstring(L, c_err.as_ptr());
            ffi::lua_setfield(L, -2, c"error".as_ptr());
        }

        // Diagnostics
        ffi::lua_createtable(L, result.diagnostics.len() as c_int, 0);
        for (i, diag) in result.diagnostics.iter().enumerate() {
            ffi::lua_createtable(L, 0, 7);

            let c_sev = CString::new(diag.severity.as_str()).unwrap();
            ffi::lua_pushstring(L, c_sev.as_ptr());
            ffi::lua_setfield(L, -2, c"severity".as_ptr());

            let c_msg = CString::new(diag.message.as_str()).unwrap();
            ffi::lua_pushstring(L, c_msg.as_ptr());
            ffi::lua_setfield(L, -2, c"message".as_ptr());

            let c_file = CString::new(diag.location.file.to_string_lossy().as_ref()).unwrap();
            ffi::lua_pushstring(L, c_file.as_ptr());
            ffi::lua_setfield(L, -2, c"file".as_ptr());

            ffi::lua_pushinteger(L, diag.location.line as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"line".as_ptr());

            ffi::lua_pushinteger(L, diag.location.column as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"column".as_ptr());

            if !diag.rule_id.is_empty() {
                let c_rule = CString::new(diag.rule_id.as_str()).unwrap();
                ffi::lua_pushstring(L, c_rule.as_ptr());
                ffi::lua_setfield(L, -2, c"rule".as_ptr());
            }

            if let Some(fix) = &diag.fix {
                ffi::lua_createtable(L, 0, 2);
                let c_desc = CString::new(fix.description.as_str()).unwrap();
                ffi::lua_pushstring(L, c_desc.as_ptr());
                ffi::lua_setfield(L, -2, c"description".as_ptr());
                let c_repl = CString::new(fix.replacement.as_str()).unwrap();
                ffi::lua_pushstring(L, c_repl.as_ptr());
                ffi::lua_setfield(L, -2, c"replacement".as_ptr());
                ffi::lua_setfield(L, -2, c"fix".as_ptr());
            }

            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        ffi::lua_setfield(L, -2, c"diagnostics".as_ptr());

        1
    }
}

/// Push an error message and call lua_error.
unsafe fn push_error(L: *mut lua_State, msg: &str) -> c_int {
    unsafe {
        let c_msg = CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap());
        ffi::lua_pushstring(L, c_msg.as_ptr());
        ffi::lua_error(L)
    }
}
