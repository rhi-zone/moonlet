//! LLM client plugin for spore.
//!
//! Provides multi-provider LLM completions:
//!
//! ## Module Functions
//! - `llm.providers()` - List available providers
//! - `llm.provider_info(name)` - Get provider details
//! - `llm.complete(provider, model?, system?, prompt, opts?)` - Single completion (blocking)
//! - `llm.chat(provider, model?, system?, prompt, history, opts?)` - Chat with history (blocking)
//! - `llm.start_chat(provider, model?, system?, prompt, history, opts?)` - Chat (async, returns Handle)

#![allow(non_snake_case)]

use mlua::ffi::{self, lua_State};
use rhizome_spore_lua::handle::{self, Handle, HandleItem, HandleResult, Stream};
use rig::{
    client::{CompletionClient, ProviderClient},
    completion::{Chat, Message},
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
    Anthropic,
    OpenAI,
    Azure,
    Gemini,
    Cohere,
    DeepSeek,
    Groq,
    Mistral,
    Ollama,
    OpenRouter,
    Perplexity,
    Together,
    XAI,
}

impl Provider {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "anthropic" | "claude" => Some(Self::Anthropic),
            "openai" | "gpt" | "chatgpt" => Some(Self::OpenAI),
            "azure" | "azure-openai" => Some(Self::Azure),
            "google" | "gemini" => Some(Self::Gemini),
            "cohere" => Some(Self::Cohere),
            "deepseek" => Some(Self::DeepSeek),
            "groq" => Some(Self::Groq),
            "mistral" => Some(Self::Mistral),
            "ollama" => Some(Self::Ollama),
            "openrouter" => Some(Self::OpenRouter),
            "perplexity" | "pplx" => Some(Self::Perplexity),
            "together" | "together-ai" => Some(Self::Together),
            "xai" | "grok" => Some(Self::XAI),
            _ => None,
        }
    }

    pub fn default_model(&self) -> &'static str {
        match self {
            Self::Anthropic => "claude-sonnet-4-5",
            Self::OpenAI => "gpt-5.2",
            Self::Azure => "gpt-5.2",
            Self::Gemini => "gemini-3-flash-preview",
            Self::Cohere => "command-r-plus",
            Self::DeepSeek => "deepseek-chat",
            Self::Groq => "moonshotai/kimi-k2-instruct-0905",
            Self::Mistral => "mistral-large-latest",
            Self::Ollama => "llama3.2",
            Self::OpenRouter => "anthropic/claude-3.5-sonnet",
            Self::Perplexity => "llama-3.1-sonar-large-128k-online",
            Self::Together => "meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo",
            Self::XAI => "grok-2-latest",
        }
    }

    pub fn env_var(&self) -> &'static str {
        match self {
            Self::Anthropic => "ANTHROPIC_API_KEY",
            Self::OpenAI => "OPENAI_API_KEY",
            Self::Azure => "AZURE_OPENAI_API_KEY",
            Self::Gemini => "GEMINI_API_KEY",
            Self::Cohere => "COHERE_API_KEY",
            Self::DeepSeek => "DEEPSEEK_API_KEY",
            Self::Groq => "GROQ_API_KEY",
            Self::Mistral => "MISTRAL_API_KEY",
            Self::Ollama => "OLLAMA_API_KEY",
            Self::OpenRouter => "OPENROUTER_API_KEY",
            Self::Perplexity => "PERPLEXITY_API_KEY",
            Self::Together => "TOGETHER_API_KEY",
            Self::XAI => "XAI_API_KEY",
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Anthropic,
            Self::OpenAI,
            Self::Azure,
            Self::Gemini,
            Self::Cohere,
            Self::DeepSeek,
            Self::Groq,
            Self::Mistral,
            Self::Ollama,
            Self::OpenRouter,
            Self::Perplexity,
            Self::Together,
            Self::XAI,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAI => "openai",
            Self::Azure => "azure",
            Self::Gemini => "gemini",
            Self::Cohere => "cohere",
            Self::DeepSeek => "deepseek",
            Self::Groq => "groq",
            Self::Mistral => "mistral",
            Self::Ollama => "ollama",
            Self::OpenRouter => "openrouter",
            Self::Perplexity => "perplexity",
            Self::Together => "together",
            Self::XAI => "xai",
        }
    }
}

// ============================================================================
// Plugin exports
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn spore_plugin_info() -> SporePluginInfo {
    SporePluginInfo {
        name: c"llm".as_ptr(),
        version: c"0.1.0".as_ptr(),
        abi_version: ABI_VERSION,
    }
}

/// Lua module entry point.
///
/// # Safety
/// Must be called from Lua with a valid lua_State pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C-unwind" fn luaopen_spore_llm(L: *mut lua_State) -> c_int {
    unsafe {
        // Register Handle metatable (from spore-lua)
        handle::register_handle_metatable(L);

        // Create module table
        ffi::lua_createtable(L, 0, 5);

        ffi::lua_pushcclosure(L, llm_providers, 0);
        ffi::lua_setfield(L, -2, c"providers".as_ptr());

        ffi::lua_pushcclosure(L, llm_provider_info, 0);
        ffi::lua_setfield(L, -2, c"provider_info".as_ptr());

        ffi::lua_pushcclosure(L, llm_complete, 0);
        ffi::lua_setfield(L, -2, c"complete".as_ptr());

        ffi::lua_pushcclosure(L, llm_chat, 0);
        ffi::lua_setfield(L, -2, c"chat".as_ptr());

        ffi::lua_pushcclosure(L, llm_start_chat, 0);
        ffi::lua_setfield(L, -2, c"start_chat".as_ptr());

        1
    }
}

// ============================================================================
// Module functions
// ============================================================================

/// llm.providers() -> array of provider names
unsafe extern "C-unwind" fn llm_providers(L: *mut lua_State) -> c_int {
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

/// llm.provider_info(name) -> info table
unsafe extern "C-unwind" fn llm_provider_info(L: *mut lua_State) -> c_int {
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

/// llm.complete(provider, model?, system?, prompt, opts?) -> response string
unsafe extern "C-unwind" fn llm_complete(L: *mut lua_State) -> c_int {
    unsafe {
        // Parse args: provider (required), model (optional), system (optional), prompt (required), opts (optional)
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "complete requires provider argument");
        }
        let provider_ptr = ffi::lua_tostring(L, 1);
        let provider_str = CStr::from_ptr(provider_ptr).to_string_lossy();

        let model = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 2);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        let system = if ffi::lua_type(L, 3) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 3);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        if ffi::lua_type(L, 4) != ffi::LUA_TSTRING {
            return push_error(L, "complete requires prompt argument");
        }
        let prompt_ptr = ffi::lua_tostring(L, 4);
        let prompt = CStr::from_ptr(prompt_ptr).to_string_lossy();

        let max_tokens = if ffi::lua_type(L, 5) == ffi::LUA_TTABLE {
            ffi::lua_getfield(L, 5, c"max_tokens".as_ptr());
            let tokens = if ffi::lua_type(L, -1) == ffi::LUA_TNUMBER {
                Some(ffi::lua_tointeger(L, -1) as usize)
            } else {
                None
            };
            ffi::lua_pop(L, 1);
            tokens
        } else {
            None
        };

        match do_complete(
            &provider_str,
            model.as_deref(),
            system.as_deref(),
            &prompt,
            max_tokens,
        ) {
            Ok(response) => {
                let c_response = CString::new(response).unwrap();
                ffi::lua_pushstring(L, c_response.as_ptr());
                1
            }
            Err(e) => push_error(L, &e),
        }
    }
}

/// llm.chat(provider, model?, system?, prompt, history, opts?) -> response string
unsafe extern "C-unwind" fn llm_chat(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "chat requires provider argument");
        }
        let provider_ptr = ffi::lua_tostring(L, 1);
        let provider_str = CStr::from_ptr(provider_ptr).to_string_lossy();

        let model = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 2);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        let system = if ffi::lua_type(L, 3) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 3);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        if ffi::lua_type(L, 4) != ffi::LUA_TSTRING {
            return push_error(L, "chat requires prompt argument");
        }
        let prompt_ptr = ffi::lua_tostring(L, 4);
        let prompt = CStr::from_ptr(prompt_ptr).to_string_lossy();

        // Parse history table
        let mut history: Vec<(String, String)> = Vec::new();
        if ffi::lua_type(L, 5) == ffi::LUA_TTABLE {
            let len = ffi::lua_rawlen(L, 5);
            for i in 1..=len {
                ffi::lua_rawgeti(L, 5, i as ffi::lua_Integer);
                if ffi::lua_type(L, -1) == ffi::LUA_TTABLE {
                    ffi::lua_getfield(L, -1, c"role".as_ptr());
                    let role = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                        CStr::from_ptr(ffi::lua_tostring(L, -1))
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        "user".to_string()
                    };
                    ffi::lua_pop(L, 1);

                    ffi::lua_getfield(L, -1, c"content".as_ptr());
                    let content = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                        CStr::from_ptr(ffi::lua_tostring(L, -1))
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        String::new()
                    };
                    ffi::lua_pop(L, 1);

                    history.push((role, content));
                }
                ffi::lua_pop(L, 1);
            }
        }

        let max_tokens = if ffi::lua_type(L, 6) == ffi::LUA_TTABLE {
            ffi::lua_getfield(L, 6, c"max_tokens".as_ptr());
            let tokens = if ffi::lua_type(L, -1) == ffi::LUA_TNUMBER {
                Some(ffi::lua_tointeger(L, -1) as usize)
            } else {
                None
            };
            ffi::lua_pop(L, 1);
            tokens
        } else {
            None
        };

        match do_chat(
            &provider_str,
            model.as_deref(),
            system.as_deref(),
            &prompt,
            history,
            max_tokens,
        ) {
            Ok(response) => {
                let c_response = CString::new(response).unwrap();
                ffi::lua_pushstring(L, c_response.as_ptr());
                1
            }
            Err(e) => push_error(L, &e),
        }
    }
}

/// llm.start_chat(provider, model?, system?, prompt, history, opts?) -> Handle
/// Starts chat asynchronously, returning a Handle for polling completion.
unsafe extern "C-unwind" fn llm_start_chat(L: *mut lua_State) -> c_int {
    unsafe {
        if ffi::lua_type(L, 1) != ffi::LUA_TSTRING {
            return push_error(L, "start_chat requires provider argument");
        }
        let provider_ptr = ffi::lua_tostring(L, 1);
        let provider_str = CStr::from_ptr(provider_ptr).to_string_lossy().into_owned();

        let model = if ffi::lua_type(L, 2) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 2);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        let system = if ffi::lua_type(L, 3) == ffi::LUA_TSTRING {
            let ptr = ffi::lua_tostring(L, 3);
            Some(CStr::from_ptr(ptr).to_string_lossy().into_owned())
        } else {
            None
        };

        if ffi::lua_type(L, 4) != ffi::LUA_TSTRING {
            return push_error(L, "start_chat requires prompt argument");
        }
        let prompt_ptr = ffi::lua_tostring(L, 4);
        let prompt = CStr::from_ptr(prompt_ptr).to_string_lossy().into_owned();

        // Parse history table
        let mut history: Vec<(String, String)> = Vec::new();
        if ffi::lua_type(L, 5) == ffi::LUA_TTABLE {
            let len = ffi::lua_rawlen(L, 5);
            for i in 1..=len {
                ffi::lua_rawgeti(L, 5, i as ffi::lua_Integer);
                if ffi::lua_type(L, -1) == ffi::LUA_TTABLE {
                    ffi::lua_getfield(L, -1, c"role".as_ptr());
                    let role = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                        CStr::from_ptr(ffi::lua_tostring(L, -1))
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        "user".to_string()
                    };
                    ffi::lua_pop(L, 1);

                    ffi::lua_getfield(L, -1, c"content".as_ptr());
                    let content = if ffi::lua_type(L, -1) == ffi::LUA_TSTRING {
                        CStr::from_ptr(ffi::lua_tostring(L, -1))
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        String::new()
                    };
                    ffi::lua_pop(L, 1);

                    history.push((role, content));
                }
                ffi::lua_pop(L, 1);
            }
        }

        let max_tokens = if ffi::lua_type(L, 6) == ffi::LUA_TTABLE {
            ffi::lua_getfield(L, 6, c"max_tokens".as_ptr());
            let tokens = if ffi::lua_type(L, -1) == ffi::LUA_TNUMBER {
                Some(ffi::lua_tointeger(L, -1) as usize)
            } else {
                None
            };
            ffi::lua_pop(L, 1);
            tokens
        } else {
            None
        };

        // Create Handle for async chat
        let handle = spawn_chat_request(provider_str, model, system, prompt, history, max_tokens);
        handle::push_handle(L, handle)
    }
}

/// Spawn an LLM chat request in a background thread and return a Handle.
fn spawn_chat_request(
    provider_str: String,
    model: Option<String>,
    system: Option<String>,
    prompt: String,
    history: Vec<(String, String)>,
    max_tokens: Option<usize>,
) -> Handle {
    let (tx, rx) = channel();
    let (kill_tx, kill_rx) = channel::<()>();

    let provider_name = provider_str.clone();

    let join_handle = std::thread::spawn(move || {
        // Check for kill signal early
        if kill_rx.try_recv().is_ok() {
            return HandleResult {
                success: false,
                exit_code: None,
                data: Some("cancelled".to_string()),
            };
        }

        // Run the blocking chat
        match do_chat(
            &provider_str,
            model.as_deref(),
            system.as_deref(),
            &prompt,
            history,
            max_tokens,
        ) {
            Ok(response) => {
                // Send the response through the channel so it can be read with :read()
                let _ = tx.send(HandleItem {
                    stream: Stream::Default,
                    content: response.clone(),
                });
                HandleResult {
                    success: true,
                    exit_code: Some(0),
                    data: Some(response),
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
// LLM implementation
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

fn do_complete(
    provider_str: &str,
    model: Option<&str>,
    system: Option<&str>,
    prompt: &str,
    max_tokens: Option<usize>,
) -> Result<String, String> {
    do_chat(provider_str, model, system, prompt, Vec::new(), max_tokens)
}

fn do_chat(
    provider_str: &str,
    model: Option<&str>,
    system: Option<&str>,
    prompt: &str,
    history: Vec<(String, String)>,
    max_tokens: Option<usize>,
) -> Result<String, String> {
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
    let max_tokens = max_tokens.unwrap_or(8192);

    // Convert history to rig Messages
    let messages: Vec<Message> = history
        .into_iter()
        .map(|(role, content)| {
            if role == "assistant" {
                Message::assistant(content)
            } else {
                Message::user(content)
            }
        })
        .collect();

    let rt =
        tokio::runtime::Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;

    rt.block_on(async {
        macro_rules! run_provider {
            ($client:expr) => {{
                let client = $client;
                let mut builder = client.agent(model_str);
                if let Some(sys) = system {
                    builder = builder.preamble(sys);
                }
                let agent = builder.build();
                agent
                    .chat(prompt, messages.clone())
                    .await
                    .map_err(|e| format!("LLM request failed: {:?}", e))
            }};
        }

        match provider {
            Provider::Anthropic => {
                let client = providers::anthropic::Client::from_env();
                let mut builder = client.agent(model_str).max_tokens(max_tokens as u64);
                if let Some(sys) = system {
                    builder = builder.preamble(sys);
                }
                let agent = builder.build();
                agent
                    .chat(prompt, messages)
                    .await
                    .map_err(|e| format!("LLM request failed: {}", e))
            }
            Provider::OpenAI => run_provider!(providers::openai::Client::from_env()),
            Provider::Azure => run_provider!(providers::azure::Client::from_env()),
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
                    run_provider!(client)
                } else {
                    run_provider!(providers::gemini::Client::from_env())
                }
            }
            Provider::Cohere => run_provider!(providers::cohere::Client::from_env()),
            Provider::DeepSeek => run_provider!(providers::deepseek::Client::from_env()),
            Provider::Groq => run_provider!(providers::groq::Client::from_env()),
            Provider::Mistral => run_provider!(providers::mistral::Client::from_env()),
            Provider::Ollama => run_provider!(providers::ollama::Client::from_env()),
            Provider::OpenRouter => {
                if should_bypass_ssl() {
                    let http_client = create_http_client()?;
                    let api_key = std::env::var("OPENROUTER_API_KEY")
                        .map_err(|_| "OPENROUTER_API_KEY not set")?;
                    let client: providers::openrouter::Client<reqwest::Client> =
                        providers::openrouter::Client::<reqwest::Client>::builder()
                            .api_key(&api_key)
                            .http_client(http_client)
                            .build()
                            .map_err(|e| format!("Failed to create OpenRouter client: {:?}", e))?;
                    run_provider!(client)
                } else {
                    run_provider!(providers::openrouter::Client::from_env())
                }
            }
            Provider::Perplexity => run_provider!(providers::perplexity::Client::from_env()),
            Provider::Together => run_provider!(providers::together::Client::from_env()),
            Provider::XAI => run_provider!(providers::xai::Client::from_env()),
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
