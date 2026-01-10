//! rhizome-spore-lotus: Lotus world integration for spore agents.
//!
//! Provides persistent multiplayer world capabilities to spore Lua scripts:
//!
//! ## Entity Operations
//! - `lotus.entity(id)` - Get entity by ID
//! - `lotus.verbs(entity)` - Get all verbs on an entity
//! - `lotus.update(id, props)` - Update entity properties
//! - `lotus.create(props, prototype?)` - Create new entity
//!
//! ## Verb Execution
//! - `lotus.call(entity, verb, args)` - Call a verb on an entity
//! - `lotus.schedule(verb, args, delay_ms)` - Schedule future verb call
//!
//! ## Capability System
//! - `lotus.capability(id)` - Get capability by ID
//! - `lotus.mint(authority, type, params)` - Mint new capability
//! - `lotus.delegate(cap, restrictions)` - Delegate with restrictions
//!
//! ## Context (available during verb execution)
//! - `lotus.this` - Current entity
//! - `lotus.caller` - Caller entity ID
//! - `lotus.args` - Arguments passed to verb

use mlua::{Lua, LuaSerdeExt, Result, Table, Value};
use rhizome_lotus_core::{Entity, EntityId, WorldStorage};
use rhizome_spore_lua::Integration;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Execution frame for nested verb calls.
#[derive(Debug, Clone)]
struct ExecutionFrame {
    /// The entity executing the verb ("this")
    this: Entity,
    /// The entity that initiated the call
    caller_id: Option<EntityId>,
    /// Arguments passed to the verb
    args: Vec<serde_json::Value>,
}

/// Thread-local execution stack for nested verb calls.
thread_local! {
    static EXECUTION_STACK: RefCell<Vec<ExecutionFrame>> = RefCell::new(Vec::new());
}

/// Lotus world integration for spore.
pub struct LotusIntegration {
    storage: Arc<Mutex<WorldStorage>>,
}

impl LotusIntegration {
    /// Create a new lotus integration with an existing storage.
    pub fn new(storage: Arc<Mutex<WorldStorage>>) -> Self {
        Self { storage }
    }

    /// Create a new lotus integration from a database path.
    pub async fn open(db_path: impl Into<PathBuf>) -> std::result::Result<Self, rhizome_lotus_core::StorageError> {
        let path = db_path.into();
        let storage = WorldStorage::open(path.to_str().unwrap_or("world.sqlite")).await?;
        Ok(Self {
            storage: Arc::new(Mutex::new(storage)),
        })
    }

    /// Get a reference to the storage.
    pub fn storage(&self) -> &Arc<Mutex<WorldStorage>> {
        &self.storage
    }
}

impl Integration for LotusIntegration {
    fn register(&self, lua: &Lua) -> Result<()> {
        let lotus = lua.create_table()?;

        // Entity operations
        register_entity(&lotus, lua, &self.storage)?;
        register_verbs(&lotus, lua, &self.storage)?;
        register_update(&lotus, lua, &self.storage)?;
        register_create(&lotus, lua, &self.storage)?;

        // Verb execution
        register_call(&lotus, lua, &self.storage)?;
        // TODO: register_schedule requires async runtime

        // Capability system
        register_capability(&lotus, lua, &self.storage)?;
        register_mint(&lotus, lua, &self.storage)?;
        register_delegate(&lotus, lua, &self.storage)?;

        // Context accessors (read from execution stack)
        register_context(&lotus, lua)?;

        lua.globals().set("lotus", lotus)?;
        Ok(())
    }
}

/// Flatten entity props to top level (matching lotus convention).
fn flatten_entity(entity: &Entity) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    result.insert("id".to_string(), serde_json::json!(entity.id));
    result.insert(
        "prototype_id".to_string(),
        serde_json::to_value(entity.prototype_id).unwrap_or(serde_json::Value::Null),
    );

    if let serde_json::Value::Object(props) = &entity.props {
        for (key, value) in props {
            result.insert(key.clone(), value.clone());
        }
    }

    serde_json::Value::Object(result)
}

/// Register lotus.entity(id) -> entity table
fn register_entity(lotus: &Table, lua: &Lua, storage: &Arc<Mutex<WorldStorage>>) -> Result<()> {
    let storage = storage.clone();
    lotus.set(
        "entity",
        lua.create_async_function(move |lua, id: i64| {
            let storage = storage.clone();
            async move {
                let storage = storage.lock().await;
                let entity = storage
                    .get_entity(id)
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?
                    .ok_or_else(|| mlua::Error::external(format!("entity {} not found", id)))?;

                let json = flatten_entity(&entity);
                lua.to_value(&json)
            }
        })?,
    )?;
    Ok(())
}

/// Register lotus.verbs(entity) -> array of verb info
fn register_verbs(lotus: &Table, lua: &Lua, storage: &Arc<Mutex<WorldStorage>>) -> Result<()> {
    let storage = storage.clone();
    lotus.set(
        "verbs",
        lua.create_async_function(move |lua, entity: Value| {
            let storage = storage.clone();
            async move {
                let entity_json: serde_json::Value = lua.from_value(entity)?;
                let entity_id = entity_json["id"]
                    .as_i64()
                    .ok_or_else(|| mlua::Error::external("verbs: entity missing id"))?;

                let storage = storage.lock().await;
                let verbs = storage
                    .get_verbs(entity_id)
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?;

                let verb_list: Vec<serde_json::Value> = verbs
                    .iter()
                    .map(|v| {
                        serde_json::json!({
                            "id": v.id,
                            "name": v.name,
                            "entity_id": v.entity_id,
                        })
                    })
                    .collect();

                lua.to_value(&verb_list)
            }
        })?,
    )?;
    Ok(())
}

/// Register lotus.update(id, props)
fn register_update(lotus: &Table, lua: &Lua, storage: &Arc<Mutex<WorldStorage>>) -> Result<()> {
    let storage = storage.clone();
    lotus.set(
        "update",
        lua.create_async_function(move |lua, (id, props): (i64, Value)| {
            let storage = storage.clone();
            async move {
                let props_json: serde_json::Value = lua.from_value(props)?;

                let storage = storage.lock().await;
                storage
                    .update_entity(id, props_json)
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?;

                Ok(())
            }
        })?,
    )?;
    Ok(())
}

/// Register lotus.create(props, prototype?) -> id
fn register_create(lotus: &Table, lua: &Lua, storage: &Arc<Mutex<WorldStorage>>) -> Result<()> {
    let storage = storage.clone();
    lotus.set(
        "create",
        lua.create_async_function(move |lua, (props, prototype_id): (Value, Option<i64>)| {
            let storage = storage.clone();
            async move {
                let props_json: serde_json::Value = lua.from_value(props)?;

                let storage = storage.lock().await;
                let id = storage
                    .create_entity(props_json, prototype_id)
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?;

                Ok(id)
            }
        })?,
    )?;
    Ok(())
}

/// Register lotus.call(entity, verb, args) -> result
fn register_call(lotus: &Table, lua: &Lua, storage: &Arc<Mutex<WorldStorage>>) -> Result<()> {
    let storage = storage.clone();
    lotus.set(
        "call",
        lua.create_async_function(move |lua, (target, verb_name, args): (Value, String, Value)| {
            let storage = storage.clone();
            async move {
                let target_json: serde_json::Value = lua.from_value(target)?;
                let target_id = target_json["id"]
                    .as_i64()
                    .ok_or_else(|| mlua::Error::external("call: target entity missing id"))?;

                let args_json: serde_json::Value = lua.from_value(args)?;
                let args_vec = match &args_json {
                    serde_json::Value::Array(arr) => arr.clone(),
                    serde_json::Value::Object(obj) if obj.is_empty() => Vec::new(),
                    serde_json::Value::Null => Vec::new(),
                    _ => return Err(mlua::Error::external("call: args must be an array")),
                };

                // Get current caller from execution stack
                let caller_id = EXECUTION_STACK.with(|stack| {
                    stack.borrow().last().map(|frame| frame.this.id)
                });

                // Get entity and verb
                let (entity, verb) = {
                    let storage = storage.lock().await;
                    let entity = storage
                        .get_entity(target_id)
                        .await
                        .map_err(|e| mlua::Error::external(e.to_string()))?
                        .ok_or_else(|| {
                            mlua::Error::external(format!("call: entity {} not found", target_id))
                        })?;
                    let verb = storage
                        .get_verb(target_id, &verb_name)
                        .await
                        .map_err(|e| mlua::Error::external(e.to_string()))?
                        .ok_or_else(|| {
                            mlua::Error::external(format!(
                                "call: verb '{}' not found on entity {}",
                                verb_name, target_id
                            ))
                        })?;

                    // Check capability requirement if set
                    if let Some(ref required_cap) = verb.required_capability {
                        if let Some(cid) = caller_id {
                            let caller_caps = storage
                                .get_capabilities(cid)
                                .await
                                .map_err(|e| mlua::Error::external(e.to_string()))?;

                            let has_required = caller_caps.iter().any(|cap| {
                                cap.cap_type == *required_cap
                                    || (cap.cap_type.ends_with(".*")
                                        && required_cap
                                            .starts_with(&cap.cap_type[..cap.cap_type.len() - 2]))
                            });

                            if !has_required {
                                return Err(mlua::Error::external(format!(
                                    "call: caller {} lacks required capability '{}' to call verb '{}'",
                                    cid, required_cap, verb_name
                                )));
                            }
                        }
                    }

                    (entity, verb)
                };

                // Push execution frame
                let frame = ExecutionFrame {
                    this: entity.clone(),
                    caller_id,
                    args: args_vec.clone(),
                };
                EXECUTION_STACK.with(|stack| {
                    stack.borrow_mut().push(frame);
                });

                // Compile verb S-expression to Lua
                // First convert lotus SExpr to serde_json::Value
                let sexpr_json: serde_json::Value = serde_json::to_value(&verb.code)
                    .map_err(|e| mlua::Error::external(format!("failed to serialize sexpr: {}", e)))?;

                // Convert to reed AST
                let ast = rhizome_reed_sexpr::from_sexpr(&sexpr_json)
                    .map_err(|e| mlua::Error::external(format!("failed to parse sexpr: {}", e)))?;

                // Generate Lua code
                let lua_code = rhizome_reed_write_lua::LuaWriter::emit(&ast);

                // Wrap with context setup
                let this_json = flatten_entity(&entity);
                let wrapped_code = format!(
                    r#"
local __this = lotus.entity({})
local __caller = {}
local __args = json.decode('{}')
local __result = (function()
{}
end)()
return __result
"#,
                    entity.id,
                    caller_id.map(|id| id.to_string()).unwrap_or_else(|| "nil".to_string()),
                    serde_json::to_string(&args_vec)
                        .unwrap()
                        .replace('\\', "\\\\")
                        .replace('\'', "\\'"),
                    lua_code
                );

                // Execute (async to handle nested async calls)
                let result: Value = lua.load(&wrapped_code).call_async(()).await?;

                // Pop execution frame
                EXECUTION_STACK.with(|stack| {
                    stack.borrow_mut().pop();
                });

                // Detect and persist __this mutations
                let this_after: serde_json::Value = lua.from_value(
                    lua.load("return __this").eval::<Value>()?
                )?;

                if this_after != this_json {
                    if let serde_json::Value::Object(after_map) = &this_after {
                        let mut updates = serde_json::Map::new();
                        let original_map = this_json.as_object().unwrap();

                        for (key, value) in after_map {
                            if key == "id" || key == "prototype_id" {
                                continue;
                            }
                            if original_map.get(key) != Some(value) {
                                updates.insert(key.clone(), value.clone());
                            }
                        }

                        if !updates.is_empty() {
                            let storage = storage.lock().await;
                            storage
                                .update_entity(entity.id, serde_json::Value::Object(updates))
                                .await
                                .map_err(|e| mlua::Error::external(e.to_string()))?;
                        }
                    }
                }

                Ok(result)
            }
        })?,
    )?;
    Ok(())
}

/// Register lotus.capability(id) -> capability table
fn register_capability(lotus: &Table, lua: &Lua, storage: &Arc<Mutex<WorldStorage>>) -> Result<()> {
    let storage = storage.clone();
    lotus.set(
        "capability",
        lua.create_async_function(move |lua, cap_id: String| {
            let storage = storage.clone();
            async move {
                let storage = storage.lock().await;
                let cap = storage
                    .get_capability(&cap_id)
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?
                    .ok_or_else(|| mlua::Error::external(format!("capability not found: {}", cap_id)))?;

                lua.to_value(&serde_json::json!({
                    "id": cap.id,
                    "owner_id": cap.owner_id,
                    "type": cap.cap_type,
                    "params": cap.params,
                }))
            }
        })?,
    )?;
    Ok(())
}

/// Register lotus.mint(authority, type, params) -> new capability
fn register_mint(lotus: &Table, lua: &Lua, storage: &Arc<Mutex<WorldStorage>>) -> Result<()> {
    let storage = storage.clone();
    lotus.set(
        "mint",
        lua.create_async_function(move |lua, (authority, cap_type, params): (Value, String, Value)| {
            let storage = storage.clone();
            async move {
                let auth_json: serde_json::Value = lua.from_value(authority)?;
                let auth_id = auth_json["id"]
                    .as_str()
                    .ok_or_else(|| mlua::Error::external("mint: authority missing id"))?
                    .to_string();

                let params_json: serde_json::Value = lua.from_value(params)?;

                // Get current entity from execution stack
                let this_id = EXECUTION_STACK.with(|stack| {
                    stack.borrow().last().map(|frame| frame.this.id)
                }).ok_or_else(|| mlua::Error::external("mint: no execution context"))?;

                let storage = storage.lock().await;

                // Validate authority
                let auth_cap = storage
                    .get_capability(&auth_id)
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?
                    .ok_or_else(|| mlua::Error::external("mint: authority capability not found"))?;

                if auth_cap.owner_id != this_id {
                    return Err(mlua::Error::external("mint: authority does not belong to this entity"));
                }

                if auth_cap.cap_type != "sys.mint" {
                    return Err(mlua::Error::external("mint: authority must be sys.mint"));
                }

                // Check namespace
                let allowed_ns = auth_cap.params
                    .get("namespace")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| mlua::Error::external("mint: authority namespace must be string"))?;

                if allowed_ns != "*" && !cap_type.starts_with(allowed_ns) {
                    return Err(mlua::Error::external(format!(
                        "mint: authority namespace '{}' does not cover '{}'",
                        allowed_ns, cap_type
                    )));
                }

                // Create new capability
                let new_id = storage
                    .create_capability(this_id, &cap_type, params_json)
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?;

                let cap = storage
                    .get_capability(&new_id)
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?
                    .ok_or_else(|| mlua::Error::external("mint: failed to retrieve new capability"))?;

                lua.to_value(&serde_json::json!({
                    "id": cap.id,
                    "type": cap.cap_type,
                    "params": cap.params,
                }))
            }
        })?,
    )?;
    Ok(())
}

/// Register lotus.delegate(cap, restrictions) -> new capability
fn register_delegate(lotus: &Table, lua: &Lua, storage: &Arc<Mutex<WorldStorage>>) -> Result<()> {
    let storage = storage.clone();
    lotus.set(
        "delegate",
        lua.create_async_function(move |lua, (parent_cap, restrictions): (Value, Value)| {
            let storage = storage.clone();
            async move {
                let parent_json: serde_json::Value = lua.from_value(parent_cap)?;
                let parent_id = parent_json["id"]
                    .as_str()
                    .ok_or_else(|| mlua::Error::external("delegate: parent capability missing id"))?
                    .to_string();

                let restrictions_json: serde_json::Value = lua.from_value(restrictions)?;
                let restrictions_obj = restrictions_json
                    .as_object()
                    .ok_or_else(|| mlua::Error::external("delegate: restrictions must be an object"))?
                    .clone();

                // Get current entity from execution stack
                let this_id = EXECUTION_STACK.with(|stack| {
                    stack.borrow().last().map(|frame| frame.this.id)
                }).ok_or_else(|| mlua::Error::external("delegate: no execution context"))?;

                let storage = storage.lock().await;

                // Get parent capability
                let parent = storage
                    .get_capability(&parent_id)
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?
                    .ok_or_else(|| mlua::Error::external("delegate: parent capability not found"))?;

                // Verify ownership
                if parent.owner_id != this_id {
                    return Err(mlua::Error::external("delegate: parent capability does not belong to this entity"));
                }

                // Merge parameters
                let mut merged_params = if let serde_json::Value::Object(map) = &parent.params {
                    map.clone()
                } else {
                    serde_json::Map::new()
                };

                for (key, value) in &restrictions_obj {
                    merged_params.insert(key.clone(), value.clone());
                }

                // Create new capability
                let new_id = storage
                    .create_capability(this_id, &parent.cap_type, serde_json::Value::Object(merged_params))
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?;

                let cap = storage
                    .get_capability(&new_id)
                    .await
                    .map_err(|e| mlua::Error::external(e.to_string()))?
                    .ok_or_else(|| mlua::Error::external("delegate: failed to retrieve new capability"))?;

                lua.to_value(&serde_json::json!({
                    "id": cap.id,
                    "type": cap.cap_type,
                    "params": cap.params,
                }))
            }
        })?,
    )?;
    Ok(())
}

/// Register context accessors: lotus.this, lotus.caller, lotus.args
fn register_context(lotus: &Table, lua: &Lua) -> Result<()> {
    // Create a metatable for lazy context access
    let mt = lua.create_table()?;

    mt.set(
        "__index",
        lua.create_function(|lua, (_, key): (Value, String)| {
            match key.as_str() {
                "this" => {
                    EXECUTION_STACK.with(|stack| {
                        if let Some(frame) = stack.borrow().last() {
                            let json = flatten_entity(&frame.this);
                            lua.to_value(&json)
                        } else {
                            Ok(Value::Nil)
                        }
                    })
                }
                "caller" => {
                    EXECUTION_STACK.with(|stack| {
                        if let Some(frame) = stack.borrow().last() {
                            Ok(frame.caller_id.map(|id| Value::Integer(id)).unwrap_or(Value::Nil))
                        } else {
                            Ok(Value::Nil)
                        }
                    })
                }
                "args" => {
                    EXECUTION_STACK.with(|stack| {
                        if let Some(frame) = stack.borrow().last() {
                            lua.to_value(&frame.args)
                        } else {
                            Ok(Value::Nil)
                        }
                    })
                }
                _ => Ok(Value::Nil),
            }
        })?,
    )?;

    lotus.set_metatable(Some(mt));
    Ok(())
}
