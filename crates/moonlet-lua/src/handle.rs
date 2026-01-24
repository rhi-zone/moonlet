#![allow(non_snake_case)] // Lua C API convention: L for lua_State

//! Async Handle infrastructure for moonlet plugins.
//!
//! Provides a Handle userdata type for managing async operations (subprocess output,
//! streaming LLM responses, etc.) with a poll-based API.
//!
//! ## Handle API (Lua)
//! ```lua
//! local h = tools:test_start("cargo")  -- or llm:start_chat(messages)
//!
//! -- Properties
//! h:is_running()  -- true/false
//!
//! -- Non-blocking read
//! h:read()        -- read next item (line or token), nil if none ready
//! h:drain()       -- drain all available items
//!
//! -- Blocking
//! h:wait()        -- block until complete, return final result
//!
//! -- Cancellation
//! h:kill()        -- terminate the operation
//! ```
//!
//! ## Poll API (Lua)
//! ```lua
//! moonlet.any_running({h1, h2})           -- true if any still running
//! moonlet.poll({h1, h2}, {timeout_ms=100}) -- handles with data ready
//! moonlet.wait_all({h1, h2})              -- wait for all, return results
//! ```

use mlua::ffi::{self, lua_State};
use std::ffi::{CString, c_char, c_int};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// Metatable name for Handle userdata.
pub const HANDLE_METATABLE: &[u8] = b"moonlet.Handle\0";

/// Stream identifier for multi-stream handles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
    /// For single-stream handles (e.g., LLM tokens)
    Default,
}

impl Stream {
    pub fn as_str(&self) -> &'static str {
        match self {
            Stream::Stdout => "stdout",
            Stream::Stderr => "stderr",
            Stream::Default => "default",
        }
    }
}

/// Result of a completed async operation.
#[derive(Debug, Clone)]
pub struct HandleResult {
    pub success: bool,
    pub exit_code: Option<i32>,
    /// Optional structured result (e.g., full LLM response)
    pub data: Option<String>,
}

/// Item received from an async operation.
#[derive(Debug, Clone)]
pub struct HandleItem {
    pub stream: Stream,
    pub content: String,
}

/// Inner state of a Handle, protected by mutex for thread safety.
struct HandleInner {
    /// Name of the operation (e.g., "cargo", "anthropic")
    name: String,
    /// Receiver for items (lines, tokens, etc.)
    receiver: Receiver<HandleItem>,
    /// Final result once operation completes
    result: Option<HandleResult>,
    /// Whether the operation is still running
    running: bool,
}

/// Handle for async operations.
///
/// This is the Rust side of the Handle userdata. It wraps:
/// - A channel receiver for streaming items
/// - A join handle for the background thread
/// - The final result once complete
pub struct Handle {
    inner: Arc<Mutex<HandleInner>>,
    /// Join handle for the background thread (if any)
    join_handle: Option<JoinHandle<HandleResult>>,
    /// Kill signal sender
    kill_tx: Option<Sender<()>>,
}

impl Handle {
    /// Create a new Handle with a receiver and optional join handle.
    pub fn new(
        name: String,
        receiver: Receiver<HandleItem>,
        join_handle: Option<JoinHandle<HandleResult>>,
        kill_tx: Option<Sender<()>>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HandleInner {
                name,
                receiver,
                result: None,
                running: true,
            })),
            join_handle,
            kill_tx,
        }
    }

    /// Create a simple Handle from just a receiver (for cases where join isn't needed).
    pub fn from_receiver(name: String, receiver: Receiver<HandleItem>) -> Self {
        Self::new(name, receiver, None, None)
    }

    /// Get the operation name.
    pub fn name(&self) -> String {
        self.inner.lock().unwrap().name.clone()
    }

    /// Check if the operation is still running.
    pub fn is_running(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();

        // If we already know it's done, return immediately
        if !inner.running {
            return false;
        }

        // Check if the channel is disconnected (sender dropped)
        match inner.receiver.try_recv() {
            Err(TryRecvError::Disconnected) => {
                inner.running = false;
                false
            }
            _ => true,
        }
    }

    /// Try to read the next item without blocking.
    pub fn try_read(&self) -> Option<HandleItem> {
        let inner = self.inner.lock().unwrap();
        inner.receiver.try_recv().ok()
    }

    /// Drain all available items.
    pub fn drain(&self) -> Vec<HandleItem> {
        let inner = self.inner.lock().unwrap();
        let mut items = Vec::new();
        while let Ok(item) = inner.receiver.try_recv() {
            items.push(item);
        }
        items
    }

    /// Wait for the operation to complete and return the result.
    pub fn wait(&mut self) -> HandleResult {
        // Drain remaining items first
        let _ = self.drain();

        // Join the thread if we have one
        if let Some(jh) = self.join_handle.take() {
            match jh.join() {
                Ok(result) => {
                    let mut inner = self.inner.lock().unwrap();
                    inner.running = false;
                    inner.result = Some(result.clone());
                    return result;
                }
                Err(_) => {
                    let mut inner = self.inner.lock().unwrap();
                    inner.running = false;
                    let result = HandleResult {
                        success: false,
                        exit_code: None,
                        data: Some("thread panicked".to_string()),
                    };
                    inner.result = Some(result.clone());
                    return result;
                }
            }
        }

        // No join handle - just mark as done and return cached result
        let mut inner = self.inner.lock().unwrap();
        inner.running = false;
        inner.result.clone().unwrap_or(HandleResult {
            success: true,
            exit_code: Some(0),
            data: None,
        })
    }

    /// Send kill signal to the operation.
    pub fn kill(&self) {
        if let Some(tx) = &self.kill_tx {
            let _ = tx.send(());
        }
        let mut inner = self.inner.lock().unwrap();
        inner.running = false;
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        // Kill on drop if still running
        self.kill();
    }
}

// ============================================================================
// Lua C API bindings
// ============================================================================

/// Register the Handle metatable.
///
/// # Safety
/// Must be called with a valid lua_State pointer.
pub unsafe fn register_handle_metatable(L: *mut lua_State) {
    unsafe {
        if ffi::luaL_newmetatable(L, HANDLE_METATABLE.as_ptr() as *const c_char) != 0 {
            ffi::lua_createtable(L, 0, 7);

            ffi::lua_pushcclosure(L, handle_is_running, 0);
            ffi::lua_setfield(L, -2, c"is_running".as_ptr());

            ffi::lua_pushcclosure(L, handle_read, 0);
            ffi::lua_setfield(L, -2, c"read".as_ptr());

            ffi::lua_pushcclosure(L, handle_drain, 0);
            ffi::lua_setfield(L, -2, c"drain".as_ptr());

            ffi::lua_pushcclosure(L, handle_wait, 0);
            ffi::lua_setfield(L, -2, c"wait".as_ptr());

            ffi::lua_pushcclosure(L, handle_kill, 0);
            ffi::lua_setfield(L, -2, c"kill".as_ptr());

            ffi::lua_pushcclosure(L, handle_name, 0);
            ffi::lua_setfield(L, -2, c"name".as_ptr());

            ffi::lua_setfield(L, -2, c"__index".as_ptr());

            ffi::lua_pushcclosure(L, handle_gc, 0);
            ffi::lua_setfield(L, -2, c"__gc".as_ptr());

            ffi::lua_pushcclosure(L, handle_tostring, 0);
            ffi::lua_setfield(L, -2, c"__tostring".as_ptr());
        }
        ffi::lua_pop(L, 1);
    }
}

/// Push a Handle as userdata onto the Lua stack.
///
/// # Safety
/// Must be called with a valid lua_State pointer.
pub unsafe fn push_handle(L: *mut lua_State, handle: Handle) -> c_int {
    unsafe {
        let boxed = Box::new(handle);
        let ud = ffi::lua_newuserdata(L, std::mem::size_of::<*mut Handle>());
        let ud_ptr = ud as *mut *mut Handle;
        *ud_ptr = Box::into_raw(boxed);

        ffi::luaL_newmetatable(L, HANDLE_METATABLE.as_ptr() as *const c_char);
        ffi::lua_setmetatable(L, -2);

        1
    }
}

unsafe fn get_handle(L: *mut lua_State, idx: c_int) -> Option<&'static mut Handle> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, idx, HANDLE_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return None;
        }
        let handle_ptr = *(ud as *const *mut Handle);
        if handle_ptr.is_null() {
            return None;
        }
        Some(&mut *handle_ptr)
    }
}

unsafe extern "C-unwind" fn handle_is_running(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(handle) = get_handle(L, 1) else {
            ffi::lua_pushboolean(L, 0);
            return 1;
        };
        ffi::lua_pushboolean(L, handle.is_running() as c_int);
        1
    }
}

unsafe extern "C-unwind" fn handle_read(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(handle) = get_handle(L, 1) else {
            ffi::lua_pushnil(L);
            return 1;
        };
        match handle.try_read() {
            Some(item) => {
                let c_content = CString::new(item.content).unwrap();
                ffi::lua_pushstring(L, c_content.as_ptr());
                1
            }
            None => {
                ffi::lua_pushnil(L);
                1
            }
        }
    }
}

unsafe extern "C-unwind" fn handle_drain(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(handle) = get_handle(L, 1) else {
            ffi::lua_createtable(L, 0, 0);
            return 1;
        };
        let items = handle.drain();
        ffi::lua_createtable(L, items.len() as c_int, 0);
        for (i, item) in items.iter().enumerate() {
            let c_content = CString::new(item.content.as_str()).unwrap();
            ffi::lua_pushstring(L, c_content.as_ptr());
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }
        1
    }
}

unsafe extern "C-unwind" fn handle_wait(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(handle) = get_handle(L, 1) else {
            ffi::lua_pushnil(L);
            return 1;
        };
        let result = handle.wait();

        ffi::lua_createtable(L, 0, 3);

        ffi::lua_pushboolean(L, result.success as c_int);
        ffi::lua_setfield(L, -2, c"success".as_ptr());

        if let Some(code) = result.exit_code {
            ffi::lua_pushinteger(L, code as ffi::lua_Integer);
            ffi::lua_setfield(L, -2, c"exit_code".as_ptr());
        }

        if let Some(data) = &result.data {
            let c_data = CString::new(data.as_str()).unwrap();
            ffi::lua_pushstring(L, c_data.as_ptr());
            ffi::lua_setfield(L, -2, c"data".as_ptr());
        }

        1
    }
}

unsafe extern "C-unwind" fn handle_kill(L: *mut lua_State) -> c_int {
    unsafe {
        if let Some(handle) = get_handle(L, 1) {
            handle.kill();
        }
        0
    }
}

unsafe extern "C-unwind" fn handle_name(L: *mut lua_State) -> c_int {
    unsafe {
        let Some(handle) = get_handle(L, 1) else {
            ffi::lua_pushstring(L, c"unknown".as_ptr());
            return 1;
        };
        let name = handle.name();
        let c_name = CString::new(name).unwrap();
        ffi::lua_pushstring(L, c_name.as_ptr());
        1
    }
}

unsafe extern "C-unwind" fn handle_gc(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1);
        if !ud.is_null() {
            let handle_ptr = *(ud as *mut *mut Handle);
            if !handle_ptr.is_null() {
                drop(Box::from_raw(handle_ptr));
            }
        }
        0
    }
}

unsafe extern "C-unwind" fn handle_tostring(L: *mut lua_State) -> c_int {
    unsafe {
        if let Some(handle) = get_handle(L, 1) {
            let running = handle.is_running();
            let name = handle.name();
            let s = format!("Handle({}, running={})", name, running);
            let c_s = CString::new(s).unwrap();
            ffi::lua_pushstring(L, c_s.as_ptr());
        } else {
            ffi::lua_pushstring(L, c"Handle(invalid)".as_ptr());
        }
        1
    }
}

// ============================================================================
// moonlet.poll, moonlet.any_running, moonlet.wait_all
// ============================================================================

/// Register moonlet.poll, moonlet.any_running, moonlet.wait_all functions.
///
/// # Safety
/// Must be called with a valid lua_State pointer after the moonlet table exists.
pub unsafe fn register_poll_functions(L: *mut lua_State) {
    unsafe {
        // Get moonlet table
        ffi::lua_getglobal(L, c"moonlet".as_ptr());
        if ffi::lua_type(L, -1) != ffi::LUA_TTABLE {
            ffi::lua_pop(L, 1);
            return;
        }

        ffi::lua_pushcclosure(L, moonlet_any_running, 0);
        ffi::lua_setfield(L, -2, c"any_running".as_ptr());

        ffi::lua_pushcclosure(L, moonlet_poll, 0);
        ffi::lua_setfield(L, -2, c"poll".as_ptr());

        ffi::lua_pushcclosure(L, moonlet_wait_all, 0);
        ffi::lua_setfield(L, -2, c"wait_all".as_ptr());

        ffi::lua_pop(L, 1);
    }
}

/// moonlet.any_running(handles) -> bool
unsafe extern "C-unwind" fn moonlet_any_running(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TTABLE {
            ffi::lua_pushboolean(L, 0);
            return 1;
        }

        let len = ffi::lua_rawlen(L, 1);
        for i in 1..=len {
            ffi::lua_rawgeti(L, 1, i as ffi::lua_Integer);
            if let Some(handle) = get_handle(L, -1)
                && handle.is_running()
            {
                ffi::lua_pop(L, 1);
                ffi::lua_pushboolean(L, 1);
                return 1;
            }
            ffi::lua_pop(L, 1);
        }

        ffi::lua_pushboolean(L, 0);
        1
    }
}

/// moonlet.poll(handles, opts?) -> array of handles with data ready
unsafe extern "C-unwind" fn moonlet_poll(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TTABLE {
            ffi::lua_createtable(L, 0, 0);
            return 1;
        }

        // Get timeout from opts
        let timeout_ms = if ffi::lua_type(L, 2) == ffi::LUA_TTABLE {
            ffi::lua_getfield(L, 2, c"timeout_ms".as_ptr());
            let ms = if ffi::lua_type(L, -1) == ffi::LUA_TNUMBER {
                ffi::lua_tointeger(L, -1) as u64
            } else {
                0
            };
            ffi::lua_pop(L, 1);
            ms
        } else {
            0
        };

        let start = std::time::Instant::now();
        let timeout = Duration::from_millis(timeout_ms);

        // Create result table
        ffi::lua_createtable(L, 0, 0);
        let mut result_idx = 1;

        loop {
            let len = ffi::lua_rawlen(L, 1);
            for i in 1..=len {
                ffi::lua_rawgeti(L, 1, i as ffi::lua_Integer);
                if let Some(handle) = get_handle(L, -1) {
                    // Check if there's data available
                    if handle.try_read().is_some() || !handle.is_running() {
                        // Copy the handle userdata to result
                        ffi::lua_pushvalue(L, -1);
                        ffi::lua_rawseti(L, -3, result_idx);
                        result_idx += 1;
                    }
                }
                ffi::lua_pop(L, 1);
            }

            // If we found any ready handles or timeout expired, return
            if result_idx > 1 || start.elapsed() >= timeout {
                break;
            }

            // Brief sleep before polling again
            std::thread::sleep(Duration::from_millis(10));
        }

        1
    }
}

/// moonlet.wait_all(handles) -> array of results
unsafe extern "C-unwind" fn moonlet_wait_all(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TTABLE {
            ffi::lua_createtable(L, 0, 0);
            return 1;
        }

        let len = ffi::lua_rawlen(L, 1);
        ffi::lua_createtable(L, len as c_int, 0);

        for i in 1..=len {
            ffi::lua_rawgeti(L, 1, i as ffi::lua_Integer);
            if let Some(handle) = get_handle(L, -1) {
                let result = handle.wait();

                ffi::lua_createtable(L, 0, 3);

                ffi::lua_pushboolean(L, result.success as c_int);
                ffi::lua_setfield(L, -2, c"success".as_ptr());

                if let Some(code) = result.exit_code {
                    ffi::lua_pushinteger(L, code as ffi::lua_Integer);
                    ffi::lua_setfield(L, -2, c"exit_code".as_ptr());
                }

                if let Some(data) = &result.data {
                    let c_data = CString::new(data.as_str()).unwrap();
                    ffi::lua_pushstring(L, c_data.as_ptr());
                    ffi::lua_setfield(L, -2, c"data".as_ptr());
                }

                ffi::lua_rawseti(L, -3, i as ffi::lua_Integer);
            }
            ffi::lua_pop(L, 1);
        }

        1
    }
}

// ============================================================================
// Helper for creating handles from subprocesses
// ============================================================================

/// Create a Handle for a subprocess with stdout/stderr streaming.
pub fn spawn_subprocess(
    name: String,
    mut command: std::process::Command,
) -> std::io::Result<Handle> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    let mut child = command.spawn()?;

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

    Ok(Handle::new(name, rx, Some(join_handle), Some(kill_tx)))
}
