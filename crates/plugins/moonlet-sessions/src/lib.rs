//! Session parsing plugin for spore.
//!
//! Provides capability-based access to AI conversation session parsing.
//!
//! ## Capability Constructor
//! - `sessions.capability({ root = "..." })` - Create sessions capability for a directory
//!
//! ## Capability Methods
//! - `cap:parse(path)` - Parse a session file into structured data
//! - `cap:parse_with_format(path, format)` - Parse with explicit format
//! - `cap:list(format?)` - List session files in capability root
//! - `cap:detect(path)` - Detect format of a session file
//!
//! ## Module Functions
//! - `sessions.formats()` - List available format names (no capability needed)

#![allow(non_snake_case)] // Lua C API convention: L for lua_State

use mlua::ffi::{self, lua_State};
use normalize_sessions::{
    ContentBlock, Message, Role, Session, SessionFile, TokenUsage, Turn, detect_format, get_format,
    list_formats, parse_session, parse_session_with_format,
};
use std::ffi::{CStr, CString, c_char, c_int};
use std::path::{Path, PathBuf};

/// Plugin ABI version.
const ABI_VERSION: u32 = 1;

/// Metatable name for SessionsCapability userdata.
const SESSIONS_CAP_METATABLE: &[u8] = b"spore.sessions.Capability\0";

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

/// Sessions capability - provides access to session parsing for a directory root.
#[derive(Debug, Clone)]
pub struct SessionsCapability {
    root: PathBuf,
}

impl SessionsCapability {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn resolve_path(&self, rel_path: &str) -> Result<PathBuf, String> {
        let path = Path::new(rel_path);
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.root.join(path)
        };

        // Canonicalize to resolve .. and symlinks
        let canonical = if full_path.exists() {
            full_path.canonicalize().map_err(|e| e.to_string())?
        } else {
            normalize_path(&full_path)
        };

        let root_canonical = if self.root.exists() {
            self.root.canonicalize().map_err(|e| e.to_string())?
        } else {
            normalize_path(&self.root)
        };

        // Ensure path doesn't escape root
        if !canonical.starts_with(&root_canonical) {
            return Err("path escapes capability root".to_string());
        }

        Ok(canonical)
    }
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                result.pop();
            }
            std::path::Component::CurDir => {}
            _ => result.push(component),
        }
    }
    result
}

// ============================================================================
// Plugin exports
// ============================================================================

/// Plugin info export.
#[unsafe(no_mangle)]
pub extern "C" fn moonlet_plugin_info() -> PluginInfo {
    PluginInfo {
        name: c"sessions".as_ptr(),
        version: c"0.1.0".as_ptr(),
        abi_version: ABI_VERSION,
    }
}

/// Lua module entry point.
///
/// # Safety
/// Must be called from Lua with a valid lua_State pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_moonlet_sessions(L: *mut lua_State) -> c_int {
    unsafe {
        // Register capability metatable
        register_capability_metatable(L);

        // Create module table
        ffi::lua_createtable(L, 0, 2);

        // sessions.capability({ root = "..." })
        ffi::lua_pushcclosure(L, sessions_capability, 0);
        ffi::lua_setfield(L, -2, c"capability".as_ptr());

        // sessions.formats() - module-level, no capability needed
        ffi::lua_pushcclosure(L, sessions_formats, 0);
        ffi::lua_setfield(L, -2, c"formats".as_ptr());

        1 // Return module table
    }
}

// ============================================================================
// Capability metatable
// ============================================================================

unsafe fn register_capability_metatable(L: *mut lua_State) {
    unsafe {
        if ffi::luaL_newmetatable(L, SESSIONS_CAP_METATABLE.as_ptr() as *const c_char) != 0 {
            // __index table with methods
            ffi::lua_createtable(L, 0, 4);

            ffi::lua_pushcclosure(L, cap_parse, 0);
            ffi::lua_setfield(L, -2, c"parse".as_ptr());

            ffi::lua_pushcclosure(L, cap_parse_with_format, 0);
            ffi::lua_setfield(L, -2, c"parse_with_format".as_ptr());

            ffi::lua_pushcclosure(L, cap_list, 0);
            ffi::lua_setfield(L, -2, c"list".as_ptr());

            ffi::lua_pushcclosure(L, cap_detect, 0);
            ffi::lua_setfield(L, -2, c"detect".as_ptr());

            ffi::lua_setfield(L, -2, c"__index".as_ptr());

            // __gc for cleanup (drop the Box)
            ffi::lua_pushcclosure(L, cap_gc, 0);
            ffi::lua_setfield(L, -2, c"__gc".as_ptr());
        }
        ffi::lua_pop(L, 1);
    }
}

/// sessions.capability({ root = "..." }) -> SessionsCapability
unsafe extern "C-unwind" fn sessions_capability(L: *mut lua_State) -> c_int {
    unsafe {
        // Expect table argument
        if ffi::lua_type(L, 1) != ffi::LUA_TTABLE {
            return push_error(L, "capability requires table argument with 'root' field");
        }

        // Get root field
        ffi::lua_getfield(L, 1, c"root".as_ptr());
        if ffi::lua_type(L, -1) != ffi::LUA_TSTRING {
            return push_error(L, "capability requires 'root' field (string)");
        }
        let root_ptr = ffi::lua_tostring(L, -1);
        let root = CStr::from_ptr(root_ptr).to_string_lossy();
        ffi::lua_pop(L, 1);

        let root_path = PathBuf::from(root.as_ref());

        // Create capability userdata
        let cap = SessionsCapability::new(root_path);
        let ud = ffi::lua_newuserdata(L, std::mem::size_of::<SessionsCapability>())
            as *mut SessionsCapability;
        std::ptr::write(ud, cap);

        // Set metatable
        ffi::luaL_setmetatable(L, SESSIONS_CAP_METATABLE.as_ptr() as *const c_char);

        1
    }
}

/// __gc metamethod
unsafe extern "C-unwind" fn cap_gc(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1) as *mut SessionsCapability;
        if !ud.is_null() {
            std::ptr::drop_in_place(ud);
        }
        0
    }
}

// ============================================================================
// Capability methods
// ============================================================================

/// Get capability from first argument (self)
unsafe fn get_capability(L: *mut lua_State) -> Result<&'static SessionsCapability, &'static str> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, 1, SESSIONS_CAP_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return Err("expected SessionsCapability");
        }
        Ok(&*(ud as *const SessionsCapability))
    }
}

/// cap:parse(path) -> Session table
unsafe extern "C-unwind" fn cap_parse(L: *mut lua_State) -> c_int {
    unsafe {
        let cap = match get_capability(L) {
            Ok(c) => c,
            Err(e) => return push_error(L, e),
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "parse requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let path = CStr::from_ptr(path_ptr).to_string_lossy();

        let resolved = match cap.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &format!("path error: {}", e)),
        };

        match parse_session(&resolved) {
            Ok(session) => push_session(L, &session),
            Err(e) => push_error(L, &format!("Parse error: {}", e)),
        }
    }
}

/// cap:parse_with_format(path, format) -> Session table
unsafe extern "C-unwind" fn cap_parse_with_format(L: *mut lua_State) -> c_int {
    unsafe {
        let cap = match get_capability(L) {
            Ok(c) => c,
            Err(e) => return push_error(L, e),
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "parse_with_format requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let path = CStr::from_ptr(path_ptr).to_string_lossy();

        if ffi::lua_type(L, 3) != ffi::LUA_TSTRING {
            return push_error(L, "parse_with_format requires format argument");
        }
        let format_ptr = ffi::lua_tostring(L, 3);
        let format = CStr::from_ptr(format_ptr).to_string_lossy();

        let resolved = match cap.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &format!("path error: {}", e)),
        };

        match parse_session_with_format(&resolved, &format) {
            Ok(session) => push_session(L, &session),
            Err(e) => push_error(L, &format!("Parse error: {}", e)),
        }
    }
}

/// cap:list(format?) -> array of {path, format, mtime}
unsafe extern "C-unwind" fn cap_list(L: *mut lua_State) -> c_int {
    unsafe {
        let cap = match get_capability(L) {
            Ok(c) => c,
            Err(e) => return push_error(L, e),
        };

        let format_name = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 2);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        ffi::lua_createtable(L, 0, 0);
        let mut idx = 1;

        if let Some(fmt_name) = format_name {
            if let Some(fmt) = get_format(&fmt_name) {
                for file in fmt.list_sessions(Some(&cap.root)) {
                    // Only include files within capability root
                    if cap.resolve_path(&file.path.to_string_lossy()).is_ok() {
                        push_session_file(L, &file, fmt.name());
                        ffi::lua_rawseti(L, -2, idx);
                        idx += 1;
                    }
                }
            }
        } else {
            for fmt_name in list_formats() {
                if let Some(fmt) = get_format(fmt_name) {
                    for file in fmt.list_sessions(Some(&cap.root)) {
                        // Only include files within capability root
                        if cap.resolve_path(&file.path.to_string_lossy()).is_ok() {
                            push_session_file(L, &file, fmt.name());
                            ffi::lua_rawseti(L, -2, idx);
                            idx += 1;
                        }
                    }
                }
            }
        }

        1
    }
}

/// cap:detect(path) -> format name or nil
unsafe extern "C-unwind" fn cap_detect(L: *mut lua_State) -> c_int {
    unsafe {
        let cap = match get_capability(L) {
            Ok(c) => c,
            Err(e) => return push_error(L, e),
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "detect requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 2);
        let path = CStr::from_ptr(path_ptr).to_string_lossy();

        let resolved = match cap.resolve_path(&path) {
            Ok(p) => p,
            Err(e) => return push_error(L, &format!("path error: {}", e)),
        };

        match detect_format(&resolved) {
            Some(fmt) => {
                let c_name = CString::new(fmt.name()).unwrap();
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

// ============================================================================
// Module functions
// ============================================================================

/// sessions.formats() -> array of format names
unsafe extern "C-unwind" fn sessions_formats(L: *mut lua_State) -> c_int {
    unsafe {
        let formats = list_formats();
        ffi::lua_createtable(L, formats.len() as c_int, 0);

        for (i, name) in formats.iter().enumerate() {
            let c_name = CString::new(*name).unwrap();
            ffi::lua_pushstring(L, c_name.as_ptr());
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }

        1
    }
}

// ============================================================================
// Conversion helpers
// ============================================================================

/// Push a Session as a Lua table.
unsafe fn push_session(L: *mut lua_State, session: &Session) -> c_int {
    unsafe {
        ffi::lua_createtable(L, 0, 6);

        // path
        let c_path = CString::new(session.path.to_string_lossy().as_ref()).unwrap();
        ffi::lua_pushstring(L, c_path.as_ptr());
        ffi::lua_setfield(L, -2, c"path".as_ptr());

        // format
        let c_format = CString::new(session.format.as_str()).unwrap();
        ffi::lua_pushstring(L, c_format.as_ptr());
        ffi::lua_setfield(L, -2, c"format".as_ptr());

        // metadata
        push_metadata(L, session);
        ffi::lua_setfield(L, -2, c"metadata".as_ptr());

        // turns
        ffi::lua_createtable(L, session.turns.len() as c_int, 0);
        for (i, turn) in session.turns.iter().enumerate() {
            push_turn(L, turn);
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        ffi::lua_setfield(L, -2, c"turns".as_ptr());

        // message_count
        ffi::lua_pushinteger(L, session.message_count() as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"message_count".as_ptr());

        // total_tokens
        let totals = session.total_tokens();
        push_token_usage(L, &totals);
        ffi::lua_setfield(L, -2, c"total_tokens".as_ptr());

        1
    }
}

/// Push session metadata.
unsafe fn push_metadata(L: *mut lua_State, session: &Session) {
    unsafe {
        ffi::lua_createtable(L, 0, 5);

        if let Some(id) = &session.metadata.session_id {
            let c_id = CString::new(id.as_str()).unwrap();
            ffi::lua_pushstring(L, c_id.as_ptr());
            ffi::lua_setfield(L, -2, c"session_id".as_ptr());
        }

        if let Some(ts) = &session.metadata.timestamp {
            let c_ts = CString::new(ts.as_str()).unwrap();
            ffi::lua_pushstring(L, c_ts.as_ptr());
            ffi::lua_setfield(L, -2, c"timestamp".as_ptr());
        }

        if let Some(provider) = &session.metadata.provider {
            let c_provider = CString::new(provider.as_str()).unwrap();
            ffi::lua_pushstring(L, c_provider.as_ptr());
            ffi::lua_setfield(L, -2, c"provider".as_ptr());
        }

        if let Some(model) = &session.metadata.model {
            let c_model = CString::new(model.as_str()).unwrap();
            ffi::lua_pushstring(L, c_model.as_ptr());
            ffi::lua_setfield(L, -2, c"model".as_ptr());
        }

        if let Some(project) = &session.metadata.project {
            let c_project = CString::new(project.as_str()).unwrap();
            ffi::lua_pushstring(L, c_project.as_ptr());
            ffi::lua_setfield(L, -2, c"project".as_ptr());
        }
    }
}

/// Push a Turn as a Lua table.
unsafe fn push_turn(L: *mut lua_State, turn: &Turn) {
    unsafe {
        ffi::lua_createtable(L, 0, 2);

        // messages
        ffi::lua_createtable(L, turn.messages.len() as c_int, 0);
        for (i, msg) in turn.messages.iter().enumerate() {
            push_message(L, msg);
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        ffi::lua_setfield(L, -2, c"messages".as_ptr());

        // token_usage
        if let Some(usage) = &turn.token_usage {
            push_token_usage(L, usage);
            ffi::lua_setfield(L, -2, c"token_usage".as_ptr());
        }
    }
}

/// Push a Message as a Lua table.
unsafe fn push_message(L: *mut lua_State, msg: &Message) {
    unsafe {
        ffi::lua_createtable(L, 0, 3);

        // role
        let role_str = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        };
        ffi::lua_pushstring(L, CString::new(role_str).unwrap().as_ptr());
        ffi::lua_setfield(L, -2, c"role".as_ptr());

        // timestamp
        if let Some(ts) = &msg.timestamp {
            let c_ts = CString::new(ts.as_str()).unwrap();
            ffi::lua_pushstring(L, c_ts.as_ptr());
            ffi::lua_setfield(L, -2, c"timestamp".as_ptr());
        }

        // content
        ffi::lua_createtable(L, msg.content.len() as c_int, 0);
        for (i, block) in msg.content.iter().enumerate() {
            push_content_block(L, block);
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        ffi::lua_setfield(L, -2, c"content".as_ptr());
    }
}

/// Push a ContentBlock as a Lua table.
unsafe fn push_content_block(L: *mut lua_State, block: &ContentBlock) {
    unsafe {
        ffi::lua_createtable(L, 0, 4);

        match block {
            ContentBlock::Text { text } => {
                ffi::lua_pushstring(L, c"text".as_ptr());
                ffi::lua_setfield(L, -2, c"type".as_ptr());

                let c_text = CString::new(text.as_str()).unwrap();
                ffi::lua_pushstring(L, c_text.as_ptr());
                ffi::lua_setfield(L, -2, c"text".as_ptr());
            }
            ContentBlock::ToolUse { id, name, input } => {
                ffi::lua_pushstring(L, c"tool_use".as_ptr());
                ffi::lua_setfield(L, -2, c"type".as_ptr());

                let c_id = CString::new(id.as_str()).unwrap();
                ffi::lua_pushstring(L, c_id.as_ptr());
                ffi::lua_setfield(L, -2, c"id".as_ptr());

                let c_name = CString::new(name.as_str()).unwrap();
                ffi::lua_pushstring(L, c_name.as_ptr());
                ffi::lua_setfield(L, -2, c"name".as_ptr());

                push_json_value(L, input);
                ffi::lua_setfield(L, -2, c"input".as_ptr());
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                ffi::lua_pushstring(L, c"tool_result".as_ptr());
                ffi::lua_setfield(L, -2, c"type".as_ptr());

                let c_id = CString::new(tool_use_id.as_str()).unwrap();
                ffi::lua_pushstring(L, c_id.as_ptr());
                ffi::lua_setfield(L, -2, c"tool_use_id".as_ptr());

                let c_content = CString::new(content.as_str()).unwrap();
                ffi::lua_pushstring(L, c_content.as_ptr());
                ffi::lua_setfield(L, -2, c"content".as_ptr());

                ffi::lua_pushboolean(L, *is_error as c_int);
                ffi::lua_setfield(L, -2, c"is_error".as_ptr());
            }
            ContentBlock::Thinking { text } => {
                ffi::lua_pushstring(L, c"thinking".as_ptr());
                ffi::lua_setfield(L, -2, c"type".as_ptr());

                let c_text = CString::new(text.as_str()).unwrap();
                ffi::lua_pushstring(L, c_text.as_ptr());
                ffi::lua_setfield(L, -2, c"text".as_ptr());
            }
        }
    }
}

/// Push TokenUsage as a Lua table.
unsafe fn push_token_usage(L: *mut lua_State, usage: &TokenUsage) {
    unsafe {
        ffi::lua_createtable(L, 0, 4);

        ffi::lua_pushinteger(L, usage.input as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"input".as_ptr());

        ffi::lua_pushinteger(L, usage.output as ffi::lua_Integer);
        ffi::lua_setfield(L, -2, c"output".as_ptr());

        if let Some(cr) = usage.cache_read {
            ffi::lua_pushinteger(L, cr as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"cache_read".as_ptr());
        }

        if let Some(cc) = usage.cache_create {
            ffi::lua_pushinteger(L, cc as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"cache_create".as_ptr());
        }
    }
}

/// Push a SessionFile as a Lua table.
unsafe fn push_session_file(L: *mut lua_State, file: &SessionFile, format: &str) {
    unsafe {
        ffi::lua_createtable(L, 0, 3);

        let c_path = CString::new(file.path.to_string_lossy().as_ref()).unwrap();
        ffi::lua_pushstring(L, c_path.as_ptr());
        ffi::lua_setfield(L, -2, c"path".as_ptr());

        let c_format = CString::new(format).unwrap();
        ffi::lua_pushstring(L, c_format.as_ptr());
        ffi::lua_setfield(L, -2, c"format".as_ptr());

        if let Ok(duration) = file.mtime.duration_since(std::time::UNIX_EPOCH) {
            ffi::lua_pushinteger(L, duration.as_secs() as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"mtime".as_ptr());
        }
    }
}

/// Push a serde_json::Value as a Lua value.
unsafe fn push_json_value(L: *mut lua_State, value: &serde_json::Value) {
    unsafe {
        match value {
            serde_json::Value::Null => {
                ffi::lua_pushnil(L);
            }
            serde_json::Value::Bool(b) => {
                ffi::lua_pushboolean(L, *b as c_int);
            }
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    ffi::lua_pushinteger(L, i);
                } else if let Some(f) = n.as_f64() {
                    ffi::lua_pushnumber(L, f);
                } else {
                    ffi::lua_pushnil(L);
                }
            }
            serde_json::Value::String(s) => {
                let c_s = CString::new(s.as_str()).unwrap();
                ffi::lua_pushstring(L, c_s.as_ptr());
            }
            serde_json::Value::Array(arr) => {
                ffi::lua_createtable(L, arr.len() as c_int, 0);
                for (i, v) in arr.iter().enumerate() {
                    push_json_value(L, v);
                    ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
                }
            }
            serde_json::Value::Object(obj) => {
                ffi::lua_createtable(L, 0, obj.len() as c_int);
                for (k, v) in obj {
                    let c_k = CString::new(k.as_str()).unwrap();
                    push_json_value(L, v);
                    ffi::lua_setfield(L, -2, c_k.as_ptr());
                }
            }
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Push an error message and call lua_error.
unsafe fn push_error(L: *mut lua_State, msg: &str) -> c_int {
    unsafe {
        let c_msg = CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap());
        ffi::lua_pushstring(L, c_msg.as_ptr());
        ffi::lua_error(L)
    }
}
