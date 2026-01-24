//! Embedding generation plugin for spore.
//!
//! Provides capability-based multi-provider embedding generation.
//!
//! ## Capability Constructor
//! - `embed.capability({ providers = {...}, models = {...} })` - Create embedding capability
//!
//! ## Capability Methods
//! - `cap:providers()` - List allowed providers
//! - `cap:provider_info(name)` - Get provider details (if allowed)
//! - `cap:generate(provider, model?, texts)` - Generate embeddings (blocking)
//! - `cap:start_generate(provider, model?, texts)` - Generate embeddings (async, returns Handle)

#![allow(non_snake_case)]

use mlua::ffi::{self, lua_State};
use moonlet_lua::handle::{self, Handle, HandleItem, HandleResult, Stream};
use rig::{
    client::{EmbeddingsClient, ProviderClient},
    embeddings::EmbeddingsBuilder,
    providers,
};
use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_int};
use std::sync::mpsc::channel;

/// Plugin ABI version.
const ABI_VERSION: u32 = 1;

/// Metatable name for EmbedCapability userdata.
const CAP_METATABLE: &[u8] = b"spore.embed.Capability\0";

/// Plugin info for version checking.
#[repr(C)]
pub struct PluginInfo {
    pub name: *const c_char,
    pub version: *const c_char,
    pub abi_version: u32,
}

// ============================================================================
// Provider enum
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    OpenAI,
    Azure,
    Gemini,
    Cohere,
    Mistral,
    Ollama,
    Together,
}

impl Provider {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "openai" | "gpt" => Some(Self::OpenAI),
            "azure" | "azure-openai" => Some(Self::Azure),
            "google" | "gemini" => Some(Self::Gemini),
            "cohere" => Some(Self::Cohere),
            "mistral" => Some(Self::Mistral),
            "ollama" => Some(Self::Ollama),
            "together" | "together-ai" => Some(Self::Together),
            _ => None,
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            Self::OpenAI => "text-embedding-3-small",
            Self::Azure => "text-embedding-3-small",
            Self::Gemini => "text-embedding-004",
            Self::Cohere => "embed-english-v3.0",
            Self::Mistral => "mistral-embed",
            Self::Ollama => "nomic-embed-text",
            Self::Together => "togethercomputer/m2-bert-80M-8k-retrieval",
        }
    }

    pub fn env_var(&self) -> &'static str {
        match self {
            Self::OpenAI => "OPENAI_API_KEY",
            Self::Azure => "AZURE_OPENAI_API_KEY",
            Self::Gemini => "GEMINI_API_KEY",
            Self::Cohere => "COHERE_API_KEY",
            Self::Mistral => "MISTRAL_API_KEY",
            Self::Ollama => "OLLAMA_API_KEY",
            Self::Together => "TOGETHER_API_KEY",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::OpenAI,
            Self::Azure,
            Self::Gemini,
            Self::Cohere,
            Self::Mistral,
            Self::Ollama,
            Self::Together,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::OpenAI => "openai",
            Self::Azure => "azure",
            Self::Gemini => "gemini",
            Self::Cohere => "cohere",
            Self::Mistral => "mistral",
            Self::Ollama => "ollama",
            Self::Together => "together",
        }
    }
}

// ============================================================================
// Capability
// ============================================================================

/// Embedding capability - provides scoped access to embedding providers.
#[derive(Debug, Clone)]
pub struct EmbedCapability {
    /// Allowed providers
    providers: Vec<Provider>,
    /// Optional model whitelist per provider (empty vec = all models allowed)
    models: HashMap<String, Vec<String>>,
}

impl EmbedCapability {
    pub fn new(providers: Vec<Provider>, models: HashMap<String, Vec<String>>) -> Self {
        Self { providers, models }
    }

    fn is_provider_allowed(&self, provider: &Provider) -> bool {
        self.providers.contains(provider)
    }

    fn is_model_allowed(&self, provider: &Provider, model: &str) -> bool {
        if let Some(allowed_models) = self.models.get(provider.name()) {
            // Empty list means all models allowed for this provider
            allowed_models.is_empty() || allowed_models.iter().any(|m| m == model)
        } else {
            // No entry means all models allowed
            true
        }
    }

    fn validate_request(&self, provider: &Provider, model: &str) -> Result<(), String> {
        if !self.is_provider_allowed(provider) {
            return Err(format!(
                "provider '{}' not allowed by capability",
                provider.name()
            ));
        }
        if !self.is_model_allowed(provider, model) {
            return Err(format!(
                "model '{}' not allowed for provider '{}'",
                model,
                provider.name()
            ));
        }
        Ok(())
    }
}

// ============================================================================
// Plugin exports
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn moonlet_plugin_info() -> PluginInfo {
    PluginInfo {
        name: c"embed".as_ptr(),
        version: c"0.1.0".as_ptr(),
        abi_version: ABI_VERSION,
    }
}

/// Lua module entry point.
///
/// # Safety
/// Must be called from Lua with a valid lua_State pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_moonlet_embed(L: *mut lua_State) -> c_int {
    unsafe {
        // Register metatables
        handle::register_handle_metatable(L);
        register_capability_metatable(L);

        // Create module table with only capability constructor
        ffi::lua_createtable(L, 0, 1);

        ffi::lua_pushcclosure(L, embed_capability, 0);
        ffi::lua_setfield(L, -2, c"capability".as_ptr());

        1
    }
}

// ============================================================================
// Capability metatable
// ============================================================================

unsafe fn register_capability_metatable(L: *mut lua_State) {
    unsafe {
        if ffi::luaL_newmetatable(L, CAP_METATABLE.as_ptr() as *const c_char) != 0 {
            // __index table with methods
            ffi::lua_createtable(L, 0, 4);

            ffi::lua_pushcclosure(L, cap_providers, 0);
            ffi::lua_setfield(L, -2, c"providers".as_ptr());

            ffi::lua_pushcclosure(L, cap_provider_info, 0);
            ffi::lua_setfield(L, -2, c"provider_info".as_ptr());

            ffi::lua_pushcclosure(L, cap_generate, 0);
            ffi::lua_setfield(L, -2, c"generate".as_ptr());

            ffi::lua_pushcclosure(L, cap_start_generate, 0);
            ffi::lua_setfield(L, -2, c"start_generate".as_ptr());

            ffi::lua_setfield(L, -2, c"__index".as_ptr());

            // __gc for cleanup
            ffi::lua_pushcclosure(L, cap_gc, 0);
            ffi::lua_setfield(L, -2, c"__gc".as_ptr());

            // __tostring for debugging
            ffi::lua_pushcclosure(L, cap_tostring, 0);
            ffi::lua_setfield(L, -2, c"__tostring".as_ptr());
        }
        ffi::lua_pop(L, 1);
    }
}

/// embed.capability({ providers = {...}, models = {...} }) -> EmbedCapability
unsafe extern "C-unwind" fn embed_capability(L: *mut lua_State) -> c_int {
    unsafe {
        // Expect table argument
        if ffi::lua_type(L, 1) != ffi::LUA_TTABLE {
            return push_error(
                L,
                "capability requires table argument with 'providers' field",
            );
        }

        // Get required providers field
        ffi::lua_getfield(L, 1, c"providers".as_ptr());
        if ffi::lua_type(L, -1) != ffi::LUA_TTABLE {
            return push_error(L, "capability requires 'providers' array");
        }

        let mut providers: Vec<Provider> = Vec::new();
        let len = ffi::lua_rawlen(L, -1);
        for i in 1..=len {
            ffi::lua_rawgeti(L, -1, i as ffi::lua_Integer);
            if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                let ptr = ffi::lua_tostring(L, -1);
                let name = CStr::from_ptr(ptr).to_string_lossy();
                if let Some(p) = Provider::parse(&name) {
                    providers.push(p);
                } else {
                    ffi::lua_pop(L, 2);
                    return push_error(L, &format!("unknown provider: {}", name));
                }
            }
            ffi::lua_pop(L, 1);
        }
        ffi::lua_pop(L, 1);

        if providers.is_empty() {
            return push_error(L, "providers array cannot be empty");
        }

        // Get optional models field
        let mut models: HashMap<String, Vec<String>> = HashMap::new();
        ffi::lua_getfield(L, 1, c"models".as_ptr());
        if ffi::lua_type(L, -1) == ffi::LUA_TTABLE {
            // Iterate over the models table (provider_name -> array of models)
            ffi::lua_pushnil(L);
            while ffi::lua_next(L, -2) != 0 {
                if ffi::lua_type(L, -2) == ffi::LUA_TSTRING {
                    let key_ptr = ffi::lua_tostring(L, -2);
                    let provider_name = CStr::from_ptr(key_ptr).to_string_lossy().into_owned();

                    let mut model_list: Vec<String> = Vec::new();
                    if ffi::lua_type(L, -1) == ffi::LUA_TTABLE {
                        let model_len = ffi::lua_rawlen(L, -1);
                        for j in 1..=model_len {
                            ffi::lua_rawgeti(L, -1, j as ffi::lua_Integer);
                            if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                                let model_ptr = ffi::lua_tostring(L, -1);
                                model_list
                                    .push(CStr::from_ptr(model_ptr).to_string_lossy().into_owned());
                            }
                            ffi::lua_pop(L, 1);
                        }
                    }
                    models.insert(provider_name, model_list);
                }
                ffi::lua_pop(L, 1);
            }
        }
        ffi::lua_pop(L, 1);

        // Create capability userdata
        let cap = EmbedCapability::new(providers, models);
        let ud =
            ffi::lua_newuserdata(L, std::mem::size_of::<EmbedCapability>()) as *mut EmbedCapability;
        std::ptr::write(ud, cap);

        // Set metatable
        ffi::luaL_setmetatable(L, CAP_METATABLE.as_ptr() as *const c_char);

        1
    }
}

/// __gc metamethod for capability
unsafe extern "C-unwind" fn cap_gc(L: *mut lua_State) -> c_int {
    unsafe {
        let ud = ffi::lua_touserdata(L, 1) as *mut EmbedCapability;
        if !ud.is_null() {
            std::ptr::drop_in_place(ud);
        }
        0
    }
}

/// __tostring metamethod for capability
unsafe extern "C-unwind" fn cap_tostring(L: *mut lua_State) -> c_int {
    unsafe {
        if let Ok(cap) = get_capability(L) {
            let provider_names: Vec<&str> = cap.providers.iter().map(|p| p.name()).collect();
            let desc = format!("EmbedCapability(providers=[{}])", provider_names.join(", "));
            let c_desc = CString::new(desc).unwrap();
            ffi::lua_pushstring(L, c_desc.as_ptr());
        } else {
            ffi::lua_pushstring(L, c"EmbedCapability(invalid)".as_ptr());
        }
        1
    }
}

/// Get capability from first argument (self)
unsafe fn get_capability(L: *mut lua_State) -> Result<&'static EmbedCapability, &'static str> {
    unsafe {
        let ud = ffi::luaL_checkudata(L, 1, CAP_METATABLE.as_ptr() as *const c_char);
        if ud.is_null() {
            return Err("expected EmbedCapability");
        }
        Ok(&*(ud as *const EmbedCapability))
    }
}

// ============================================================================
// Capability methods
// ============================================================================

/// cap:providers() -> array of allowed provider names
unsafe extern "C-unwind" fn cap_providers(L: *mut lua_State) -> c_int {
    unsafe {
        let cap = match get_capability(L) {
            Ok(c) => c,
            Err(e) => return push_error(L, e),
        };

        ffi::lua_createtable(L, cap.providers.len() as c_int, 0);

        for (i, p) in cap.providers.iter().enumerate() {
            let c_name = CString::new(p.name()).unwrap();
            ffi::lua_pushstring(L, c_name.as_ptr());
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }

        1
    }
}

/// cap:provider_info(name) -> info table
unsafe extern "C-unwind" fn cap_provider_info(L: *mut lua_State) -> c_int {
    unsafe {
        let cap = match get_capability(L) {
            Ok(c) => c,
            Err(e) => return push_error(L, e),
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "provider_info requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 2);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        match Provider::parse(&name) {
            Some(p) => {
                if !cap.is_provider_allowed(&p) {
                    return push_error(L, &format!("provider '{}' not allowed", name));
                }

                ffi::lua_createtable(L, 0, 3);

                let c_name = CString::new(p.name()).unwrap();
                ffi::lua_pushstring(L, c_name.as_ptr());
                ffi::lua_setfield(L, -2, c"name".as_ptr());

                let c_model = CString::new(p.default_model()).unwrap();
                ffi::lua_pushstring(L, c_model.as_ptr());
                ffi::lua_setfield(L, -2, c"default_model".as_ptr());

                let c_env = CString::new(p.env_var()).unwrap();
                ffi::lua_pushstring(L, c_env.as_ptr());
                ffi::lua_setfield(L, -2, c"env_var".as_ptr());

                1
            }
            None => push_error(L, &format!("unknown provider: {}", name)),
        }
    }
}

/// cap:generate(provider, model?, texts) -> array of embeddings
unsafe extern "C-unwind" fn cap_generate(L: *mut lua_State) -> c_int {
    unsafe {
        let cap = match get_capability(L) {
            Ok(c) => c,
            Err(e) => return push_error(L, e),
        };

        // Parse args: provider (required), model (optional), texts (required array)
        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "generate requires provider argument");
        }
        let provider_ptr = ffi::lua_tostring(L, 2);
        let provider_str = CStr::from_ptr(provider_ptr).to_string_lossy();

        let provider = match Provider::parse(&provider_str) {
            Some(p) => p,
            None => return push_error(L, &format!("unknown provider: {}", provider_str)),
        };

        let model = if ffi::lua_type(L, 3) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 3);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        // texts is arg 4 (or 3 if model was nil)
        let texts_idx = if model.is_some() { 4 } else { 3 };

        if ffi::lua_type(L, texts_idx) != ffi::LUA_TTABLE {
            return push_error(L, "generate requires texts array");
        }

        let mut texts: Vec<String> = Vec::new();
        let len = ffi::lua_rawlen(L, texts_idx);
        for i in 1..=len {
            ffi::lua_rawgeti(L, texts_idx, i as ffi::lua_Integer);
            if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                let ptr = ffi::lua_tostring(L, -1);
                texts.push(CStr::from_ptr(ptr).to_string_lossy().into_owned());
            }
            ffi::lua_pop(L, 1);
        }

        if texts.is_empty() {
            return push_error(L, "texts array is empty");
        }

        let model_str = model
            .as_deref()
            .unwrap_or(provider.default_model())
            .to_string();

        // Validate against capability
        if let Err(e) = cap.validate_request(&provider, &model_str) {
            return push_error(L, &e);
        }

        match do_generate(&provider_str, Some(&model_str), texts) {
            Ok(embeddings) => {
                // Return array of embedding arrays
                ffi::lua_createtable(L, embeddings.len() as c_int, 0);
                for (i, embedding) in embeddings.iter().enumerate() {
                    ffi::lua_createtable(L, embedding.len() as c_int, 0);
                    for (j, val) in embedding.iter().enumerate() {
                        ffi::lua_pushnumber(L, *val as f64);
                        ffi::lua_rawseti(L, -2, (j + 1) as ffi::lua_Integer);
                    }
                    ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
                }
                1
            }
            Err(e) => push_error(L, &e),
        }
    }
}

/// cap:start_generate(provider, model?, texts) -> Handle
unsafe extern "C-unwind" fn cap_start_generate(L: *mut lua_State) -> c_int {
    unsafe {
        let cap = match get_capability(L) {
            Ok(c) => c,
            Err(e) => return push_error(L, e),
        };

        if ffi::lua_type(L, 2) != ffi::LUA_TSTRING {
            return push_error(L, "start_generate requires provider argument");
        }
        let provider_ptr = ffi::lua_tostring(L, 2);
        let provider_str = CStr::from_ptr(provider_ptr).to_string_lossy().into_owned();

        let provider = match Provider::parse(&provider_str) {
            Some(p) => p,
            None => return push_error(L, &format!("unknown provider: {}", provider_str)),
        };

        let model = if ffi::lua_type(L, 3) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 3);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        let texts_idx = if model.is_some() { 4 } else { 3 };

        if ffi::lua_type(L, texts_idx) != ffi::LUA_TTABLE {
            return push_error(L, "start_generate requires texts array");
        }

        let mut texts: Vec<String> = Vec::new();
        let len = ffi::lua_rawlen(L, texts_idx);
        for i in 1..=len {
            ffi::lua_rawgeti(L, texts_idx, i as ffi::lua_Integer);
            if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                let ptr = ffi::lua_tostring(L, -1);
                texts.push(CStr::from_ptr(ptr).to_string_lossy().into_owned());
            }
            ffi::lua_pop(L, 1);
        }

        if texts.is_empty() {
            return push_error(L, "texts array is empty");
        }

        let model_str = model
            .clone()
            .unwrap_or_else(|| provider.default_model().to_string());

        // Validate against capability
        if let Err(e) = cap.validate_request(&provider, &model_str) {
            return push_error(L, &e);
        }

        let handle = spawn_generate_request(provider_str, Some(model_str), texts);
        handle::push_handle(L, handle)
    }
}

/// Spawn an embedding request in a background thread and return a Handle.
fn spawn_generate_request(
    provider_str: String,
    model: Option<String>,
    texts: Vec<String>,
) -> Handle {
    let (tx, rx) = channel();
    let (kill_tx, kill_rx) = channel::<()>();

    let provider_name = provider_str.clone();

    let join_handle = std::thread::spawn(move || {
        if kill_rx.try_recv().is_ok() {
            return HandleResult {
                success: false,
                exit_code: None,
                data: Some("cancelled".to_string()),
            };
        }

        match do_generate(&provider_str, model.as_deref(), texts) {
            Ok(embeddings) => {
                // Serialize embeddings to JSON for the result
                let json = serde_json::to_string(&embeddings).unwrap_or_default();
                let _ = tx.send(HandleItem {
                    stream: Stream::Default,
                    content: json.clone(),
                });
                HandleResult {
                    success: true,
                    exit_code: Some(0),
                    data: Some(json),
                }
            }
            Err(e) => {
                let _ = tx.send(HandleItem {
                    stream: Stream::Stderr,
                    content: e.clone(),
                });
                HandleResult {
                    success: false,
                    exit_code: Some(1),
                    data: Some(e),
                }
            }
        }
    });

    Handle::new(provider_name, rx, Some(join_handle), Some(kill_tx))
}

// ============================================================================
// Embedding implementation
// ============================================================================

fn should_bypass_ssl() -> bool {
    std::env::var("SPORE_INSECURE_SSL")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

fn create_http_client() -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder();

    if should_bypass_ssl() {
        eprintln!("WARNING: SSL certificate validation disabled (SPORE_INSECURE_SSL=1)");
        builder = builder.danger_accept_invalid_certs(true);
    }

    builder
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

fn do_generate(
    provider_str: &str,
    model: Option<&str>,
    texts: Vec<String>,
) -> Result<Vec<Vec<f32>>, String> {
    let provider = Provider::parse(provider_str).ok_or_else(|| {
        format!(
            "Unsupported provider: {}. Available: {}",
            provider_str,
            Provider::all()
                .iter()
                .map(|p| p.name())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;

    // Check for API key (ollama is optional)
    if provider != Provider::Ollama && std::env::var(provider.env_var()).is_err() {
        return Err(format!(
            "Missing {} environment variable for {} provider",
            provider.env_var(),
            provider_str
        ));
    }

    let model_str = model.unwrap_or(provider.default_model());

    let rt =
        tokio::runtime::Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;

    rt.block_on(async {
        // Helper to extract vectors from embedding results
        // The builder returns Vec<(String, OneOrMany<Embedding>)>
        fn extract_vectors(
            results: Vec<(
                String,
                rig::one_or_many::OneOrMany<rig::embeddings::Embedding>,
            )>,
        ) -> Vec<Vec<f32>> {
            results
                .into_iter()
                .flat_map(|(_, embeddings)| {
                    embeddings
                        .into_iter()
                        .map(|e| e.vec.iter().map(|&v| v as f32).collect())
                })
                .collect()
        }

        macro_rules! run_embedding {
            ($client:expr) => {{
                let model = $client.embedding_model(model_str);
                let mut builder = EmbeddingsBuilder::new(model);
                for text in &texts {
                    builder = builder
                        .document(text.clone())
                        .map_err(|e| format!("Failed to add document: {}", e))?;
                }
                let results = builder
                    .build()
                    .await
                    .map_err(|e| format!("Embedding request failed: {:?}", e))?;
                Ok(extract_vectors(results))
            }};
        }

        macro_rules! run_cohere_embedding {
            ($client:expr) => {{
                let model = $client.embedding_model(model_str, "search_document");
                let mut builder = EmbeddingsBuilder::new(model);
                for text in &texts {
                    builder = builder
                        .document(text.clone())
                        .map_err(|e| format!("Failed to add document: {}", e))?;
                }
                let results = builder
                    .build()
                    .await
                    .map_err(|e| format!("Embedding request failed: {:?}", e))?;
                Ok(extract_vectors(results))
            }};
        }

        match provider {
            Provider::OpenAI => run_embedding!(providers::openai::Client::from_env()),
            Provider::Azure => run_embedding!(providers::azure::Client::from_env()),
            Provider::Gemini => {
                if should_bypass_ssl() {
                    let http_client = create_http_client()?;
                    let api_key =
                        std::env::var("GEMINI_API_KEY").map_err(|_| "GEMINI_API_KEY not set")?;
                    let client: providers::gemini::Client<reqwest::Client> =
                        providers::gemini::Client::<reqwest::Client>::builder()
                            .api_key(&api_key)
                            .http_client(http_client)
                            .build()
                            .map_err(|e| format!("Failed to create Gemini client: {:?}", e))?;
                    run_embedding!(client)
                } else {
                    run_embedding!(providers::gemini::Client::from_env())
                }
            }
            Provider::Cohere => run_cohere_embedding!(providers::cohere::Client::from_env()),
            Provider::Mistral => run_embedding!(providers::mistral::Client::from_env()),
            Provider::Ollama => run_embedding!(providers::ollama::Client::from_env()),
            Provider::Together => run_embedding!(providers::together::Client::from_env()),
        }
    })
}

// ============================================================================
// Helpers
// ============================================================================

unsafe fn push_error(L: *mut lua_State, msg: &str) -> c_int {
    unsafe {
        let c_msg = CString::new(msg).unwrap_or_else(|_| CString::new("error").unwrap());
        ffi::lua_pushstring(L, c_msg.as_ptr());
        ffi::lua_error(L)
    }
}
