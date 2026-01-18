//! Embedding generation plugin for spore.
//!
//! Provides multi-provider embedding generation:
//!
//! ## Module Functions
//! - `embed.providers()` - List available providers with embedding support
//! - `embed.provider_info(name)` - Get provider details and default model
//! - `embed.generate(provider, model?, texts)` - Generate embeddings (blocking)
//! - `embed.start_generate(provider, model?, texts)` - Generate embeddings (async, returns Handle)

#![allow(non_snake_case)]

use mlua::ffi::{self, lua_State};
use rhizome_spore_lua::handle::{self, Handle, HandleItem, HandleResult, Stream};
use rig::{
    client::{EmbeddingsClient, ProviderClient},
    embeddings::EmbeddingsBuilder,
    providers,
};
use std::ffi::{CStr, CString, c_char, c_int};
use std::sync::mpsc::channel;

/// Plugin ABI version.
const ABI_VERSION: u32 = 1;

/// Plugin info for version checking.
#[repr(C)]
pub struct SporePluginInfo {
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
// Plugin exports
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn spore_plugin_info() -> SporePluginInfo {
    SporePluginInfo {
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
pub unsafe extern "C-unwind" fn luaopen_spore_embed(L: *mut lua_State) -> c_int {
    unsafe {
        // Register Handle metatable (from spore-lua)
        handle::register_handle_metatable(L);

        // Create module table
        ffi::lua_createtable(L, 0, 4);

        ffi::lua_pushcclosure(L, embed_providers, 0);
        ffi::lua_setfield(L, -2, c"providers".as_ptr());

        ffi::lua_pushcclosure(L, embed_provider_info, 0);
        ffi::lua_setfield(L, -2, c"provider_info".as_ptr());

        ffi::lua_pushcclosure(L, embed_generate, 0);
        ffi::lua_setfield(L, -2, c"generate".as_ptr());

        ffi::lua_pushcclosure(L, embed_start_generate, 0);
        ffi::lua_setfield(L, -2, c"start_generate".as_ptr());

        1
    }
}

// ============================================================================
// Module functions
// ============================================================================

/// embed.providers() -> array of provider names
unsafe extern "C-unwind" fn embed_providers(L: *mut lua_State) -> c_int {
    unsafe {
        let providers = Provider::all();
        ffi::lua_createtable(L, providers.len() as c_int, 0);

        for (i, p) in providers.iter().enumerate() {
            let c_name = CString::new(p.name()).unwrap();
            ffi::lua_pushstring(L, c_name.as_ptr());
            ffi::lua_rawseti(L, -2, (i + 1) as ffi::lua_Integer);
        }

        1
    }
}

/// embed.provider_info(name) -> info table
unsafe extern "C-unwind" fn embed_provider_info(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "provider_info requires name argument");
        }
        let name_ptr = ffi::lua_tostring(L, 1);
        let name = CStr::from_ptr(name_ptr).to_string_lossy();

        match Provider::parse(&name) {
            Some(p) => {
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
            None => push_error(L, &format!("Unknown provider: {}", name)),
        }
    }
}

/// embed.generate(provider, model?, texts) -> array of embeddings
unsafe extern "C-unwind" fn embed_generate(L: *mut lua_State) -> c_int {
    unsafe {
        // Parse args: provider (required), model (optional), texts (required array)
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "generate requires provider argument");
        }
        let provider_ptr = ffi::lua_tostring(L, 1);
        let provider_str = CStr::from_ptr(provider_ptr).to_string_lossy();

        let model = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 2);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        // texts is arg 3 (or 2 if model was nil)
        let texts_idx = if model.is_some() { 3 } else { 2 };

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

        match do_generate(&provider_str, model.as_deref(), texts) {
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

/// embed.start_generate(provider, model?, texts) -> Handle
/// Starts embedding generation asynchronously, returning a Handle for polling completion.
unsafe extern "C-unwind" fn embed_start_generate(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "start_generate requires provider argument");
        }
        let provider_ptr = ffi::lua_tostring(L, 1);
        let provider_str = CStr::from_ptr(provider_ptr).to_string_lossy().into_owned();

        let model = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 2);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        let texts_idx = if model.is_some() { 3 } else { 2 };

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

        let handle = spawn_generate_request(provider_str, model, texts);
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
