//! rhizome-spore-moss-sessions: Moss sessions integration for spore agents.
//!
//! Registers session parsing functions into the spore Lua runtime:
//!
//! ## Parsing
//! - `sessions.parse(path)` - Parse a session file into structured data
//! - `sessions.parse_with_format(path, format)` - Parse with explicit format
//!
//! ## Discovery
//! - `sessions.list(project?, format?)` - List session files
//! - `sessions.formats()` - List available format names
//! - `sessions.detect(path)` - Detect format of a session file

use mlua::{Lua, Result, Table, Value};
use rhizome_moss_sessions::{
    ContentBlock, Message, Role, Session, SessionFile, TokenUsage, Turn, detect_format, get_format,
    list_formats, parse_session, parse_session_with_format,
};
use rhizome_spore_lua::Integration;
use std::path::Path;

/// Moss sessions integration for spore.
pub struct MossSessionsIntegration;

impl Integration for MossSessionsIntegration {
    fn register(&self, lua: &Lua) -> Result<()> {
        let sessions = lua.create_table()?;

        // sessions.parse(path) -> Session table
        sessions.set(
            "parse",
            lua.create_function(|lua, path: String| {
                let session = parse_session(Path::new(&path))
                    .map_err(|e| mlua::Error::external(format!("Parse error: {}", e)))?;
                session_to_lua(lua, &session)
            })?,
        )?;

        // sessions.parse_with_format(path, format) -> Session table
        sessions.set(
            "parse_with_format",
            lua.create_function(|lua, (path, format): (String, String)| {
                let session = parse_session_with_format(Path::new(&path), &format)
                    .map_err(|e| mlua::Error::external(format!("Parse error: {}", e)))?;
                session_to_lua(lua, &session)
            })?,
        )?;

        // sessions.list(project?, format?) -> array of {path, format, mtime}
        sessions.set(
            "list",
            lua.create_function(|lua, args: (Option<String>, Option<String>)| {
                let (project, format_name) = args;
                let project_path = project.as_deref().map(Path::new);

                let result = lua.create_table()?;
                let mut idx = 1;

                if let Some(fmt_name) = format_name {
                    // List for specific format
                    if let Some(fmt) = get_format(&fmt_name) {
                        for file in fmt.list_sessions(project_path) {
                            result.set(idx, session_file_to_lua(lua, &file, fmt.name())?)?;
                            idx += 1;
                        }
                    }
                } else {
                    // List for all formats
                    for fmt_name in list_formats() {
                        if let Some(fmt) = get_format(fmt_name) {
                            for file in fmt.list_sessions(project_path) {
                                result.set(idx, session_file_to_lua(lua, &file, fmt.name())?)?;
                                idx += 1;
                            }
                        }
                    }
                }
                Ok(result)
            })?,
        )?;

        // sessions.formats() -> array of format names
        sessions.set(
            "formats",
            lua.create_function(|lua, ()| {
                let formats = list_formats();
                let result = lua.create_table()?;
                for (i, name) in formats.iter().enumerate() {
                    result.set(i + 1, *name)?;
                }
                Ok(result)
            })?,
        )?;

        // sessions.detect(path) -> format name or nil
        sessions.set(
            "detect",
            lua.create_function(|_, path: String| {
                Ok(detect_format(Path::new(&path)).map(|f| f.name()))
            })?,
        )?;

        lua.globals().set("sessions", sessions)?;
        Ok(())
    }
}

/// Convert a Session to a Lua table
fn session_to_lua(lua: &Lua, session: &Session) -> Result<Table> {
    let t = lua.create_table()?;

    t.set("path", session.path.to_string_lossy().to_string())?;
    t.set("format", session.format.clone())?;

    // Metadata
    let meta = lua.create_table()?;
    if let Some(id) = &session.metadata.session_id {
        meta.set("session_id", id.clone())?;
    }
    if let Some(ts) = &session.metadata.timestamp {
        meta.set("timestamp", ts.clone())?;
    }
    if let Some(provider) = &session.metadata.provider {
        meta.set("provider", provider.clone())?;
    }
    if let Some(model) = &session.metadata.model {
        meta.set("model", model.clone())?;
    }
    if let Some(project) = &session.metadata.project {
        meta.set("project", project.clone())?;
    }
    t.set("metadata", meta)?;

    // Turns
    let turns = lua.create_table()?;
    for (i, turn) in session.turns.iter().enumerate() {
        turns.set(i + 1, turn_to_lua(lua, turn)?)?;
    }
    t.set("turns", turns)?;

    // Helper stats
    t.set("message_count", session.message_count())?;
    let totals = session.total_tokens();
    let tokens = lua.create_table()?;
    tokens.set("input", totals.input)?;
    tokens.set("output", totals.output)?;
    if let Some(cr) = totals.cache_read {
        tokens.set("cache_read", cr)?;
    }
    if let Some(cc) = totals.cache_create {
        tokens.set("cache_create", cc)?;
    }
    t.set("total_tokens", tokens)?;

    Ok(t)
}

/// Convert a Turn to a Lua table
fn turn_to_lua(lua: &Lua, turn: &Turn) -> Result<Table> {
    let t = lua.create_table()?;

    let messages = lua.create_table()?;
    for (i, msg) in turn.messages.iter().enumerate() {
        messages.set(i + 1, message_to_lua(lua, msg)?)?;
    }
    t.set("messages", messages)?;

    if let Some(usage) = &turn.token_usage {
        t.set("token_usage", token_usage_to_lua(lua, usage)?)?;
    }

    Ok(t)
}

/// Convert a Message to a Lua table
fn message_to_lua(lua: &Lua, msg: &Message) -> Result<Table> {
    let t = lua.create_table()?;

    t.set(
        "role",
        match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::System => "system",
        },
    )?;

    if let Some(ts) = &msg.timestamp {
        t.set("timestamp", ts.clone())?;
    }

    let content = lua.create_table()?;
    for (i, block) in msg.content.iter().enumerate() {
        content.set(i + 1, content_block_to_lua(lua, block)?)?;
    }
    t.set("content", content)?;

    Ok(t)
}

/// Convert a ContentBlock to a Lua table
fn content_block_to_lua(lua: &Lua, block: &ContentBlock) -> Result<Table> {
    let t = lua.create_table()?;

    match block {
        ContentBlock::Text { text } => {
            t.set("type", "text")?;
            t.set("text", text.clone())?;
        }
        ContentBlock::ToolUse { id, name, input } => {
            t.set("type", "tool_use")?;
            t.set("id", id.clone())?;
            t.set("name", name.clone())?;
            // Convert serde_json::Value to Lua
            let input_lua = json_to_lua(lua, input)?;
            t.set("input", input_lua)?;
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => {
            t.set("type", "tool_result")?;
            t.set("tool_use_id", tool_use_id.clone())?;
            t.set("content", content.clone())?;
            t.set("is_error", *is_error)?;
        }
        ContentBlock::Thinking { text } => {
            t.set("type", "thinking")?;
            t.set("text", text.clone())?;
        }
    }

    Ok(t)
}

/// Convert TokenUsage to a Lua table
fn token_usage_to_lua(lua: &Lua, usage: &TokenUsage) -> Result<Table> {
    let t = lua.create_table()?;
    t.set("input", usage.input)?;
    t.set("output", usage.output)?;
    if let Some(cr) = usage.cache_read {
        t.set("cache_read", cr)?;
    }
    if let Some(cc) = usage.cache_create {
        t.set("cache_create", cc)?;
    }
    Ok(t)
}

/// Convert a SessionFile to a Lua table
fn session_file_to_lua(lua: &Lua, file: &SessionFile, format: &str) -> Result<Table> {
    let t = lua.create_table()?;
    t.set("path", file.path.to_string_lossy().to_string())?;
    t.set("format", format)?;
    // Convert SystemTime to unix timestamp
    if let Ok(duration) = file.mtime.duration_since(std::time::UNIX_EPOCH) {
        t.set("mtime", duration.as_secs())?;
    }
    Ok(t)
}

/// Convert serde_json::Value to Lua Value
fn json_to_lua(lua: &Lua, value: &serde_json::Value) -> Result<Value> {
    match value {
        serde_json::Value::Null => Ok(Value::Nil),
        serde_json::Value::Bool(b) => Ok(Value::Boolean(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Number(f))
            } else {
                Ok(Value::Nil)
            }
        }
        serde_json::Value::String(s) => Ok(Value::String(lua.create_string(s)?)),
        serde_json::Value::Array(arr) => {
            let t = lua.create_table()?;
            for (i, v) in arr.iter().enumerate() {
                t.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(t))
        }
        serde_json::Value::Object(obj) => {
            let t = lua.create_table()?;
            for (k, v) in obj {
                t.set(k.clone(), json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(t))
        }
    }
}
