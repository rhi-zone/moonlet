//! LLM client for workflow engine.
//!
//! Supports all providers from rig: anthropic, openai, google, cohere, groq, etc.

#[cfg(feature = "llm")]
use rig::{
    client::{CompletionClient, ProviderClient},
    completion::{Chat, Message},
    providers,
};

#[cfg(feature = "llm")]
use reqwest;

#[cfg(feature = "llm")]
/// Check if SSL certificate validation should be bypassed
fn should_bypass_ssl() -> bool {
    std::env::var("SPORE_INSECURE_SSL")
        .map(|v| v == "1" || v.to_lowercase() == "true")
        .unwrap_or(false)
}

#[cfg(feature = "llm")]
/// Create a reqwest client, optionally with SSL verification disabled
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

/// Supported LLM providers.
#[cfg(feature = "llm")]
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

#[cfg(feature = "llm")]
impl Provider {
    /// Parse provider from string.
    pub fn from_str(s: &str) -> Option<Self> {
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

    /// Get default model for this provider.
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

    /// Get environment variable name for API key.
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

    /// List all providers.
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
}

/// LLM client.
#[cfg(feature = "llm")]
pub struct LlmClient {
    provider: Provider,
    model: String,
}

#[cfg(feature = "llm")]
impl LlmClient {
    /// Create a new LLM client.
    pub fn new(provider_str: &str, model: Option<&str>) -> Result<Self, String> {
        let provider = Provider::from_str(provider_str).ok_or_else(|| {
            format!(
                "Unsupported provider: {}. Available: {}",
                provider_str,
                Provider::all()
                    .iter()
                    .map(|p| format!("{:?}", p).to_lowercase())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

        // Check for API key (ollama is optional since it can be local)
        if provider != Provider::Ollama && std::env::var(provider.env_var()).is_err() {
            return Err(format!(
                "Missing {} environment variable for {} provider",
                provider.env_var(),
                provider_str
            ));
        }

        let model = model
            .map(|m| m.to_string())
            .unwrap_or_else(|| provider.default_model().to_string());

        Ok(Self { provider, model })
    }

    /// Generate a completion.
    pub fn complete(
        &self,
        system: Option<&str>,
        prompt: &str,
        max_tokens: Option<usize>,
    ) -> Result<String, String> {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| format!("Failed to create runtime: {}", e))?;
        rt.block_on(self.complete_async(system, prompt, max_tokens.unwrap_or(8192)))
    }

    async fn complete_async(
        &self,
        system: Option<&str>,
        prompt: &str,
        max_tokens: usize,
    ) -> Result<String, String> {
        self.chat_async(system, prompt, Vec::new(), max_tokens)
            .await
    }

    /// Chat with message history.
    pub fn chat(
        &self,
        system: Option<&str>,
        prompt: &str,
        history: Vec<(String, String)>, // (role, content) pairs
        max_tokens: Option<usize>,
    ) -> Result<String, String> {
        let rt = tokio::runtime::Runtime::new()
            .map_err(|e| format!("Failed to create runtime: {}", e))?;
        rt.block_on(self.chat_async(system, prompt, history, max_tokens.unwrap_or(8192)))
    }

    async fn chat_async(
        &self,
        system: Option<&str>,
        prompt: &str,
        history: Vec<(String, String)>,
        max_tokens: usize,
    ) -> Result<String, String> {
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

        macro_rules! run_provider {
            ($client:expr) => {{
                let client = $client;
                let mut builder = client.agent(&self.model);
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

        match self.provider {
            Provider::Anthropic => {
                // Anthropic requires max_tokens
                let client = providers::anthropic::Client::from_env();
                let mut builder = client.agent(&self.model).max_tokens(max_tokens as u64);
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
                // Create custom HTTP client for SSL bypass if needed
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
                // Create custom HTTP client for SSL bypass if needed
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "llm")]
    #[test]
    fn test_provider_parsing() {
        assert_eq!(Provider::from_str("anthropic"), Some(Provider::Anthropic));
        assert_eq!(Provider::from_str("claude"), Some(Provider::Anthropic));
        assert_eq!(Provider::from_str("openai"), Some(Provider::OpenAI));
        assert_eq!(Provider::from_str("gpt"), Some(Provider::OpenAI));
        assert_eq!(Provider::from_str("google"), Some(Provider::Gemini));
        assert_eq!(Provider::from_str("gemini"), Some(Provider::Gemini));
        assert_eq!(Provider::from_str("groq"), Some(Provider::Groq));
        assert_eq!(Provider::from_str("ollama"), Some(Provider::Ollama));
        assert_eq!(Provider::from_str("unknown"), None);
    }

    #[cfg(feature = "llm")]
    #[test]
    fn test_all_providers_have_defaults() {
        for provider in Provider::all() {
            assert!(!provider.default_model().is_empty());
            assert!(!provider.env_var().is_empty());
        }
    }
}
