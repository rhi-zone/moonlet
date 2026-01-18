//! LibSQL/SQLite plugin for spore with vector support.
//!
//! Provides direct database access with native vector operations:
//!
//! ## Module Functions
//! - `libsql.open(path)` - Open a database connection (returns Connection)
//! - `libsql.open_memory()` - Open an in-memory database
//! - `libsql.vector32(array)` - Format a Lua array as vector32 SQL literal
//! - `libsql.vector64(array)` - Format a Lua array as vector64 SQL literal
//!
//! ## Connection Methods
//! - `conn:execute(sql, params?)` - Execute SQL, returns rows affected
//! - `conn:query(sql, params?)` - Query SQL, returns array of row tables
//! - `conn:close()` - Close the connection

#![allow(non_snake_case)]

use mlua::ffi::{self, lua_State};
use std::ffi::{CStr, CString, c_char, c_int};
use std::sync::{Arc, Mutex};

/// Plugin ABI version.
const ABI_VERSION: u32 = 1;

/// Metatable name for Connection userdata.
const CONN_METATABLE: &[u8] = b"spore.libsql.Connection\0";

/// Plugin info for version checking.
#[repr(C)]
pub struct SporePluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub abi_version: u32,
}

// ============================================================================
// Connection wrapper
// ============================================================================

/// Wrapper around libsql Connection that can be shared with Lua.
struct Connection {
    conn: Arc<Mutex<Option<libsql::Connection>>>,
}

impl Connection {
    fn new(conn: libsql::Connection) -> Self {
        Self {
            conn: Arc::new(Mutex::new(Some(conn))),
        }
    }

    fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&libsql::Connection) -> Result<T, String>,
    {
        let guard = self.conn.lock().map_err(|e| e.to_string())?;
        match guard.as_ref() {
            Some(conn) => f(conn),
            None => Err("connection is closed".to_string()),
        }
    }

    fn close(&self) -> Result<(), String> {
        let mut guard = self.conn.lock().map_err(|e| e.to_string())?;
        *guard = None;
        Ok(())
    }
}

// ============================================================================
// Plugin exports
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn spore_plugin_info() -> SporePluginInfo {
    SporePluginInfo {
        name: c"libsql".as_ptr(),
        version: c"0.1.0".as_ptr(),
        abi_version: ABI_VERSION,
    }
}

/// Lua module entry point.
///
/// # Safety
/// Must be called from Lua with a valid lua_State pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_spore_libsql(L: *mut lua_State) -> c_int {
    unsafe {
        // Register connection metatable
        register_connection_metatable(L);

        // Create module table
        ffi::lua_createtable(L, 0, 4);

        ffi::lua_pushcclosure(L, libsql_open, 0);
        ffi::lua_setfield(L, -2, c"open".as_ptr());

        ffi::lua_pushcclosure(L, libsql_open_memory, 0);
        ffi::lua_setfield(L, -2, c"open_memory".as_ptr());

        ffi::lua_pushcclosure(L, libsql_vector32, 0);
        ffi::lua_setfield(L, -2, c"vector32".as_ptr());

        ffi::lua_pushcclosure(L, libsql_vector64, 0);
        ffi::lua_setfield(L, -2, c"vector64".as_ptr());

        1
    }
}

// ============================================================================
// Connection metatable
// ============================================================================

unsafe fn register_connection_metatable(L: *mut lua_State) {
    unsafe {
        if ffi::luaL_newmetatable(L, CONN_METATABLE.as_ptr() as *const c_char) != 0 {
            ffi::lua_createtable(L, 0, 3);

            ffi::lua_pushcclosure(L, conn_execute, 0);
            ffi::lua_setfield(L, -2, c"execute".as_ptr());

            ffi::lua_pushcclosure(L, conn_query, 0);
            ffi::lua_setfield(L, -2, c"query".as_ptr());

            ffi::lua_pushcclosure(L, conn_close, 0);
            ffi::lua_setfield(L, -2, c"close".as_ptr());

            ffi::lua_setfield(L, -2, c"__index".as_ptr());

            ffi::lua_pushcclosure(L, conn_gc, 0);
            ffi::lua_setfield(L, -2, c"__gc".as_ptr());

            ffi::lua_pushcclosure(L, conn_tostring, 0);
            ffi::lua_setfield(L, -2, c"__tostring".as_ptr());
        }
        ffi::lua_pop(L, 1);
    }
}

// ============================================================================
// Module functions
// ============================================================================

/// libsql.open(path) -> Connection
unsafe extern "C-unwind" fn libsql_open(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "open requires path argument");
        }
        let path_ptr = ffi::lua_tostring(L, 1);
        let path = CStr::from_ptr(path_ptr).to_string_lossy();

        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => return push_error(L, &format!("Failed to create runtime: {}", e)),
        };

        let result = rt.block_on(async {
            let db = libsql::Builder::new_local(&*path)
                .build()
                .await
                .map_err(|e| format!("Failed to open database: {}", e))?;
            let conn = db
                .connect()
                .map_err(|e| format!("Failed to connect: {}", e))?;
            Ok::<_, String>(conn)
        });

        match result {
            Ok(conn) => create_connection_userdata(L, Connection::new(conn)),
            Err(e) => push_error(L, &e),
        }
    }
}

/// libsql.open_memory() -> Connection
unsafe extern "C-unwind" fn libsql_open_memory(L: *mut lua_State) -> c_int {
    unsafe {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => return push_error(L, &format!("Failed to create runtime: {}", e)),
        };

        let result = rt.block_on(async {
            let db = libsql::Builder::new_local(":memory:")
                .build()
                .await
                .map_err(|e| format!("Failed to open database: {}", e))?;
            let conn = db
                .connect()
                .map_err(|e| format!("Failed to connect: {}", e))?;
            Ok::<_, String>(conn)
        });

        match result {
            Ok(conn) => create_connection_userdata(L, Connection::new(conn)),
            Err(e) => push_error(L, &e),
        }
    }
}

/// libsql.vector32(array) -> string formatted as vector32('[...]')
unsafe extern "C-unwind" fn libsql_vector32(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TTABLE {
            return push_error(L, "vector32 requires array argument");
        }

        let mut values: Vec<f32> = Vec::new();
        let len = ffi::lua_rawlen(L, 1);
        for i in 1..=len {
            ffi::lua_rawgeti(L, 1, i as ffi::lua_Integer);
            if ffi::lua_type(L, -1) == ffi::LUA_TNUMBER {
                values.push(ffi::lua_tonumber(L, -1) as f32);
            }
            ffi::lua_pop(L, 1);
        }

        let json = format!(
            "[{}]",
            values
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let result = format!("vector32('{}')", json);
        let c_result = CString::new(result).unwrap();
        ffi::lua_pushstring(L, c_result.as_ptr());
        1
    }
}

/// libsql.vector64(array) -> string formatted as vector64('[...]')
unsafe extern "C-unwind" fn libsql_vector64(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TTABLE {
            return push_error(L, "vector64 requires array argument");
        }

        let mut values: Vec<f64> = Vec::new();
        let len = ffi::lua_rawlen(L, 1);
        for i in 1..=len {
            ffi::lua_rawgeti(L, 1, i as ffi::lua_Integer);
            if ffi::lua_type(L, -1) == ffi::LUA_TNUMBER {
                values.push(ffi::lua_tonumber(L, -1));
            }
            ffi::lua_pop(L, 1);
        }

        let json = format!(
            "[{}]",
            values
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let result = format!("vector64('{}')", json);
        let c_result = CString::new(result).unwrap();
        ffi::lua_pushstring(L, c_result.as_ptr());
        1
    }
}

// ============================================================================
// Connection methods
// ============================================================================

unsafe fn create_connection_userdata(L: *mut lua_State, conn: Connection) -> c_int {
    unsafe {
        let boxed = Box::new(conn);
        let ud = ffi::lua_newuserdata(L, std::mem::size_of::<*mut Connection>());
        let ud_ptr = ud as *mut *mut Connection;
        *ud_ptr = Box::into_raw(boxed);

        ffi::luaL_newmetatable(L, CONN_METATABLE.as_ptr() as *const c_char);
        ffi::lua_setmetatable(L, -2);

        1
    }
}

unsafe fn get_connection(L: *mut lua_State, idx: c_int) -> Option<&'static Connection> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, idx, CONN_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return None;
        }
        let conn_ptr = *(ud as *const *mut Connection);
        if conn_ptr.is_null() {
            return None;
        }
        Some(&*conn_ptr)
    }
}

/// conn:execute(sql, params?) -> rows_affected
unsafe extern "C-unwind" fn conn_execute(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(conn) = get_connection(L, 1) else {
            return push_error(L, "invalid connection");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "execute requires sql argument");
        }
        let sql_ptr = ffi::lua_tostring(L, 2);
        let sql = CStr::from_ptr(sql_ptr).to_string_lossy().into_owned();

        // Parse optional params
        let params = parse_params(L, 3);

        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => return push_error(L, &format!("Failed to create runtime: {}", e)),
        };

        let result = conn.with_conn(|c| {
            rt.block_on(async {
                let result = c
                    .execute(&sql, params)
                    .await
                    .map_err(|e| format!("Execute failed: {}", e))?;
                Ok(result)
            })
        });

        match result {
            Ok(rows) => {
                ffi::lua_pushinteger(L, rows as ffi::lua_Integer);
                1
            }
            Err(e) => push_error(L, &e),
        }
    }
}

/// conn:query(sql, params?) -> array of row tables
unsafe extern "C-unwind" fn conn_query(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(conn) = get_connection(L, 1) else {
            return push_error(L, "invalid connection");
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "query requires sql argument");
        }
        let sql_ptr = ffi::lua_tostring(L, 2);
        let sql = CStr::from_ptr(sql_ptr).to_string_lossy().into_owned();

        // Parse optional params
        let params = parse_params(L, 3);

        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => return push_error(L, &format!("Failed to create runtime: {}", e)),
        };

        let result = conn.with_conn(|c| {
            rt.block_on(async {
                let mut rows = c
                    .query(&sql, params)
                    .await
                    .map_err(|e| format!("Query failed: {}", e))?;

                let mut results: Vec<Vec<(String, libsql::Value)>> = Vec::new();
                while let Some(row) = rows.next().await.map_err(|e| e.to_string())? {
                    let col_count = rows.column_count();
                    let mut row_data = Vec::new();
                    for i in 0..col_count {
                        let name = rows
                            .column_name(i)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("col{}", i));
                        let value = row.get_value(i).map_err(|e| e.to_string())?;
                        row_data.push((name, value));
                    }
                    results.push(row_data);
                }
                Ok(results)
            })
        });

        match result {
            Ok(rows) => {
                ffi::lua_createtable(L, rows.len() as c_int, 0);
                for (i, row) in rows.iter().enumerate() {
                    ffi::lua_createtable(L, 0, row.len() as c_int);
                    for (name, value) in row {
                        push_value(L, value);
                        let c_name = CString::new(name.as_str()).unwrap();
                        ffi::lua_setfield(L, -2, c_name.as_ptr());
                    }
                    ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
                }
                1
            }
            Err(e) => push_error(L, &e),
        }
    }
}

/// conn:close()
unsafe extern "C-unwind" fn conn_close(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(conn) = get_connection(L, 1) else {
            return push_error(L, "invalid connection");
        };

        if let Err(e) = conn.close() {
            return push_error(L, &e);
        }

        0
    }
}

unsafe extern "C-unwind" fn conn_gc(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1);
        if !ud.is_null() {
            let conn_ptr = *(ud as *mut *mut Connection);
            if !conn_ptr.is_null() {
                drop(Box::from_raw(conn_ptr));
            }
        }
        0
    }
}

unsafe extern "C-unwind" fn conn_tostring(L: *mut lua_State) -> c_int {
    unsafe {
        if get_connection(L, 1).is_some() {
            ffi::lua_pushstring(L, c"LibsqlConnection(open)".as_ptr());
        } else {
            ffi::lua_pushstring(L, c"LibsqlConnection(closed)".as_ptr());
        }
        1
    }
}

// ============================================================================
// Helpers
// ============================================================================

unsafe fn parse_params(L: *mut lua_State, idx: c_int) -> Vec<libsql::Value> {
    unsafe {
        let mut params = Vec::new();
        if ffi::lua_type(L, idx) == ffi::LUA_TTABLE {
            let len = ffi::lua_rawlen(L, idx);
            for i in 1..=len {
                ffi::lua_rawgeti(L, idx, i as ffi::lua_Integer);
                let value = lua_to_libsql_value(L, -1);
                params.push(value);
                ffi::lua_pop(L, 1);
            }
        }
        params
    }
}

unsafe fn lua_to_libsql_value(L: *mut lua_State, idx: c_int) -> libsql::Value {
    unsafe {
        match ffi::lua_type(L, idx) {
            ffi::LUA_TNIL => libsql::Value::Null,
            ffi::LUA_TBOOLEAN => {
                let b = ffi::lua_toboolean(L, idx) != 0;
                libsql::Value::Integer(if b { 1 } else { 0 })
            }
            ffi::LUA_TNUMBER => {
                let n = ffi::lua_tonumber(L, idx);
                // Check if it's an integer
                if n.fract() == 0.0 && n >= i64::MIN as f64 && n <= i64::MAX as f64 {
                    libsql::Value::Integer(n as i64)
                } else {
                    libsql::Value::Real(n)
                }
            }
            ffi::LUA_TSTRING => {
                let ptr = ffi::lua_tostring(L, idx);
                let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
                libsql::Value::Text(s)
            }
            _ => libsql::Value::Null,
        }
    }
}

unsafe fn push_value(L: *mut lua_State, value: &libsql::Value) {
    unsafe {
        match value {
            libsql::Value::Null => ffi::lua_pushnil(L),
            libsql::Value::Integer(i) => ffi::lua_pushinteger(L, *i as ffi::lua_Integer),
            libsql::Value::Real(r) => ffi::lua_pushnumber(L, *r),
            libsql::Value::Text(s) => {
                let c_s = CString::new(s.as_str()).unwrap();
                ffi::lua_pushstring(L, c_s.as_ptr());
            }
            libsql::Value::Blob(b) => {
                // Push blob as string (Lua doesn't distinguish)
                ffi::lua_pushlstring(L, b.as_ptr() as *const c_char, b.len());
            }
        }
    }
}

unsafe fn push_error(L: *mut lua_State, msg: &str) -> c_int {
    unsafe {
        let c_msg = CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap());
        ffi::lua_pushstring(L, c_msg.as_ptr());
        ffi::lua_error(L)
    }
}
