//! spore CLI - Run Lua scripts with plugin integrations.

use clap::{Parser, Subcommand};
use rhizome_spore_lua::{RequireConfig, Runtime};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "spore", about = "Lua runtime with plugin integrations")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run the entry script
    Run {
        /// Project directory (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,

        /// Override the entry point script
        #[arg(short, long)]
        entry: Option<PathBuf>,

        /// Arguments to pass to the Lua script
        #[arg(last = true)]
        args: Vec<String>,
    },

    /// Initialize a new spore project
    Init {
        /// Project directory (defaults to current directory)
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Config {
    project: ProjectConfig,
    #[serde(default)]
    plugins: PluginsConfig,
    #[serde(default)]
    sandbox: SandboxConfig,
    #[serde(default)]
    caps: CapsConfig,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SandboxConfig {
    /// Allow require() for Lua builtins (string, table, math, etc.)
    /// These always take precedence and cannot be overridden.
    #[serde(default = "default_true")]
    require_builtins: bool,
    /// Allow require() for loaded spore plugins (e.g., require("spore.sessions"))
    #[serde(default = "default_true")]
    require_plugins: bool,
    /// Allow require() for project Lua modules (relative to project root)
    #[serde(default = "default_true")]
    require_project: bool,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            require_builtins: true,
            require_plugins: true,
            require_project: true,
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProjectConfig {
    entry: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct PluginsConfig {
    #[serde(default)]
    fs: bool,
    #[serde(default)]
    llm: bool,
    #[serde(default)]
    embed: bool,
    #[serde(default)]
    libsql: bool,
    #[serde(default)]
    moss: bool,
    #[serde(default)]
    sessions: bool,
    #[serde(default)]
    tools: bool,
    #[serde(default)]
    packages: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct CapsConfig {
    #[serde(default)]
    fs: std::collections::HashMap<String, FsCapConfig>,
    #[serde(default)]
    moss: std::collections::HashMap<String, MossCapConfig>,
    #[serde(default)]
    sessions: std::collections::HashMap<String, SessionsCapConfig>,
    #[serde(default)]
    tools: std::collections::HashMap<String, ToolsCapConfig>,
    #[serde(default)]
    packages: std::collections::HashMap<String, PackagesCapConfig>,
    #[serde(default)]
    llm: std::collections::HashMap<String, LlmCapConfig>,
    #[serde(default)]
    embed: std::collections::HashMap<String, EmbedCapConfig>,
    #[serde(default)]
    libsql: std::collections::HashMap<String, LibsqlCapConfig>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FsCapConfig {
    path: String,
    #[serde(default = "default_mode")]
    mode: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct MossCapConfig {
    root: String,
    #[serde(default = "default_rw_mode")]
    mode: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SessionsCapConfig {
    root: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ToolsCapConfig {
    root: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PackagesCapConfig {
    root: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LlmCapConfig {
    providers: Vec<String>,
    #[serde(default)]
    models: std::collections::HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct EmbedCapConfig {
    providers: Vec<String>,
    #[serde(default)]
    models: std::collections::HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct LibsqlCapConfig {
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    allow_memory: bool,
}

fn default_mode() -> String {
    "r".to_string()
}

fn default_rw_mode() -> String {
    "rw".to_string()
}

/// Handle --schema flag for Nursery integration.
fn handle_schema_flag() -> bool {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(|s| s.as_str()) == Some("--schema") {
        let response = serde_json::json!({
            "config_path": ".spore/config.toml",
            "format": "toml",
            "schema": schemars::schema_for!(Config)
        });
        println!("{}", serde_json::to_string_pretty(&response).unwrap());
        true
    } else {
        false
    }
}

fn main() {
    // Handle --schema for Nursery integration (before clap parsing)
    if handle_schema_flag() {
        return;
    }

    let cli = Cli::parse();

    match cli.command {
        Commands::Run { path, entry, args } => {
            if let Err(e) = run(&path, entry.as_deref(), &args) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Init { path } => {
            if let Err(e) = init_project(&path) {
                eprintln!("error: {}", e);
                std::process::exit(1);
            }
        }
    }
}

fn run(project_path: &Path, entry_override: Option<&Path>, args: &[String]) -> Result<(), String> {
    let project_path = project_path
        .canonicalize()
        .map_err(|e| format!("Invalid project path: {}", e))?;

    // Load config
    let config_path = project_path.join(".spore/config.toml");
    if !config_path.exists() {
        return Err("No .spore/config.toml found. Run 'spore init' first.".to_string());
    }

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config: {}", e))?;
    let config: Config =
        toml::from_str(&content).map_err(|e| format!("Failed to parse config: {}", e))?;

    // Determine entry point
    let entry = entry_override
        .map(PathBuf::from)
        .unwrap_or_else(|| project_path.join(&config.project.entry));

    if !entry.exists() {
        return Err(format!("Entry point not found: {}", entry.display()));
    }

    // Create runtime
    let mut runtime = Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;

    // Add project-local plugin path
    runtime.add_plugin_path(project_path.join(".spore/plugins"));

    // Load plugins based on config
    if config.plugins.fs {
        runtime
            .load_plugin("fs")
            .map_err(|e| format!("Failed to load fs plugin: {}", e))?;
    }

    if config.plugins.llm {
        runtime
            .load_plugin("llm")
            .map_err(|e| format!("Failed to load llm plugin: {}", e))?;
    }

    if config.plugins.embed {
        runtime
            .load_plugin("embed")
            .map_err(|e| format!("Failed to load embed plugin: {}", e))?;
    }

    if config.plugins.libsql {
        runtime
            .load_plugin("libsql")
            .map_err(|e| format!("Failed to load libsql plugin: {}", e))?;
    }

    if config.plugins.moss {
        runtime
            .load_plugin("moss")
            .map_err(|e| format!("Failed to load moss plugin: {}", e))?;
    }

    if config.plugins.sessions {
        runtime
            .load_plugin("sessions")
            .map_err(|e| format!("Failed to load sessions plugin: {}", e))?;
    }

    if config.plugins.tools {
        runtime
            .load_plugin("tools")
            .map_err(|e| format!("Failed to load tools plugin: {}", e))?;
    }

    if config.plugins.packages {
        runtime
            .load_plugin("packages")
            .map_err(|e| format!("Failed to load packages plugin: {}", e))?;
    }

    // Set project root and args in Lua
    let lua = runtime.lua();
    let spore_table = lua
        .globals()
        .get::<mlua::Table>("spore")
        .map_err(|e| format!("Failed to get spore table: {}", e))?;

    spore_table
        .set("root", project_path.to_string_lossy().to_string())
        .map_err(|e| format!("Failed to set project root: {}", e))?;

    // Set spore.args as a Lua table
    let args_table = lua
        .create_table()
        .map_err(|e| format!("Failed to create args table: {}", e))?;
    for (i, arg) in args.iter().enumerate() {
        args_table
            .set(i + 1, arg.as_str())
            .map_err(|e| format!("Failed to set arg: {}", e))?;
    }
    spore_table
        .set("args", args_table)
        .map_err(|e| format!("Failed to set args: {}", e))?;

    // Create capabilities from config
    let caps = create_capabilities(&runtime, &config.caps, &project_path)?;

    // Create require config from sandbox settings
    let require_config = RequireConfig {
        builtins: config.sandbox.require_builtins,
        plugins: config.sandbox.require_plugins,
        project: config.sandbox.require_project,
        project_root: Some(project_path.clone()),
    };

    // Run the entry point with capabilities
    let code = std::fs::read_to_string(&entry)
        .map_err(|e| format!("Failed to read entry script: {}", e))?;

    runtime
        .run_with_caps(&code, caps, &require_config)
        .map_err(|e| format!("Script error: {}", e))?;

    Ok(())
}

/// Create capabilities from config.
fn create_capabilities(
    runtime: &Runtime,
    caps_config: &CapsConfig,
    project_path: &Path,
) -> Result<mlua::Table, String> {
    let lua = runtime.lua();
    let caps = lua
        .create_table()
        .map_err(|e| format!("Failed to create caps table: {}", e))?;

    // Create fs capabilities
    if !caps_config.fs.is_empty() {
        let fs_caps = lua
            .create_table()
            .map_err(|e| format!("Failed to create fs caps table: {}", e))?;

        for (name, fs_config) in &caps_config.fs {
            let expanded_path = expand_path(&fs_config.path, project_path);

            let params = lua
                .create_table()
                .map_err(|e| format!("Failed to create params: {}", e))?;
            params
                .set("path", expanded_path)
                .map_err(|e| format!("Failed to set path: {}", e))?;
            params
                .set("mode", fs_config.mode.clone())
                .map_err(|e| format!("Failed to set mode: {}", e))?;

            let cap = runtime
                .create_capability("fs", params)
                .map_err(|e| format!("Failed to create fs capability '{}': {}", name, e))?;

            fs_caps
                .set(name.as_str(), cap)
                .map_err(|e| format!("Failed to set capability: {}", e))?;
        }

        caps.set("fs", fs_caps)
            .map_err(|e| format!("Failed to set fs caps: {}", e))?;
    }

    // Create moss capabilities
    if !caps_config.moss.is_empty() {
        let moss_caps = lua
            .create_table()
            .map_err(|e| format!("Failed to create moss caps table: {}", e))?;

        for (name, moss_config) in &caps_config.moss {
            let expanded_root = expand_path(&moss_config.root, project_path);

            let params = lua
                .create_table()
                .map_err(|e| format!("Failed to create params: {}", e))?;
            params
                .set("root", expanded_root)
                .map_err(|e| format!("Failed to set root: {}", e))?;
            params
                .set("mode", moss_config.mode.clone())
                .map_err(|e| format!("Failed to set mode: {}", e))?;

            let cap = runtime
                .create_capability("moss", params)
                .map_err(|e| format!("Failed to create moss capability '{}': {}", name, e))?;

            moss_caps
                .set(name.as_str(), cap)
                .map_err(|e| format!("Failed to set capability: {}", e))?;
        }

        caps.set("moss", moss_caps)
            .map_err(|e| format!("Failed to set moss caps: {}", e))?;
    }

    // Create sessions capabilities
    if !caps_config.sessions.is_empty() {
        let sessions_caps = lua
            .create_table()
            .map_err(|e| format!("Failed to create sessions caps table: {}", e))?;

        for (name, sessions_config) in &caps_config.sessions {
            let expanded_root = expand_path(&sessions_config.root, project_path);

            let params = lua
                .create_table()
                .map_err(|e| format!("Failed to create params: {}", e))?;
            params
                .set("root", expanded_root)
                .map_err(|e| format!("Failed to set root: {}", e))?;

            let cap = runtime
                .create_capability("sessions", params)
                .map_err(|e| format!("Failed to create sessions capability '{}': {}", name, e))?;

            sessions_caps
                .set(name.as_str(), cap)
                .map_err(|e| format!("Failed to set capability: {}", e))?;
        }

        caps.set("sessions", sessions_caps)
            .map_err(|e| format!("Failed to set sessions caps: {}", e))?;
    }

    // Create tools capabilities
    if !caps_config.tools.is_empty() {
        let tools_caps = lua
            .create_table()
            .map_err(|e| format!("Failed to create tools caps table: {}", e))?;

        for (name, tools_config) in &caps_config.tools {
            let expanded_root = expand_path(&tools_config.root, project_path);

            let params = lua
                .create_table()
                .map_err(|e| format!("Failed to create params: {}", e))?;
            params
                .set("root", expanded_root)
                .map_err(|e| format!("Failed to set root: {}", e))?;

            let cap = runtime
                .create_capability("tools", params)
                .map_err(|e| format!("Failed to create tools capability '{}': {}", name, e))?;

            tools_caps
                .set(name.as_str(), cap)
                .map_err(|e| format!("Failed to set capability: {}", e))?;
        }

        caps.set("tools", tools_caps)
            .map_err(|e| format!("Failed to set tools caps: {}", e))?;
    }

    // Create packages capabilities
    if !caps_config.packages.is_empty() {
        let packages_caps = lua
            .create_table()
            .map_err(|e| format!("Failed to create packages caps table: {}", e))?;

        for (name, packages_config) in &caps_config.packages {
            let expanded_root = expand_path(&packages_config.root, project_path);

            let params = lua
                .create_table()
                .map_err(|e| format!("Failed to create params: {}", e))?;
            params
                .set("root", expanded_root)
                .map_err(|e| format!("Failed to set root: {}", e))?;

            let cap = runtime
                .create_capability("packages", params)
                .map_err(|e| format!("Failed to create packages capability '{}': {}", name, e))?;

            packages_caps
                .set(name.as_str(), cap)
                .map_err(|e| format!("Failed to set capability: {}", e))?;
        }

        caps.set("packages", packages_caps)
            .map_err(|e| format!("Failed to set packages caps: {}", e))?;
    }

    // Create llm capabilities
    if !caps_config.llm.is_empty() {
        let llm_caps = lua
            .create_table()
            .map_err(|e| format!("Failed to create llm caps table: {}", e))?;

        for (name, llm_config) in &caps_config.llm {
            let params = lua
                .create_table()
                .map_err(|e| format!("Failed to create params: {}", e))?;

            // Convert providers Vec to Lua table
            let providers_table = lua
                .create_table()
                .map_err(|e| format!("Failed to create providers table: {}", e))?;
            for (i, provider) in llm_config.providers.iter().enumerate() {
                providers_table
                    .set(i + 1, provider.as_str())
                    .map_err(|e| format!("Failed to set provider: {}", e))?;
            }
            params
                .set("providers", providers_table)
                .map_err(|e| format!("Failed to set providers: {}", e))?;

            // Convert models HashMap to Lua table
            let models_table = lua
                .create_table()
                .map_err(|e| format!("Failed to create models table: {}", e))?;
            for (provider, model_list) in &llm_config.models {
                let model_array = lua
                    .create_table()
                    .map_err(|e| format!("Failed to create model array: {}", e))?;
                for (i, model) in model_list.iter().enumerate() {
                    model_array
                        .set(i + 1, model.as_str())
                        .map_err(|e| format!("Failed to set model: {}", e))?;
                }
                models_table
                    .set(provider.as_str(), model_array)
                    .map_err(|e| format!("Failed to set model list: {}", e))?;
            }
            params
                .set("models", models_table)
                .map_err(|e| format!("Failed to set models: {}", e))?;

            let cap = runtime
                .create_capability("llm", params)
                .map_err(|e| format!("Failed to create llm capability '{}': {}", name, e))?;

            llm_caps
                .set(name.as_str(), cap)
                .map_err(|e| format!("Failed to set capability: {}", e))?;
        }

        caps.set("llm", llm_caps)
            .map_err(|e| format!("Failed to set llm caps: {}", e))?;
    }

    // Create embed capabilities
    if !caps_config.embed.is_empty() {
        let embed_caps = lua
            .create_table()
            .map_err(|e| format!("Failed to create embed caps table: {}", e))?;

        for (name, embed_config) in &caps_config.embed {
            let params = lua
                .create_table()
                .map_err(|e| format!("Failed to create params: {}", e))?;

            // Convert providers Vec to Lua table
            let providers_table = lua
                .create_table()
                .map_err(|e| format!("Failed to create providers table: {}", e))?;
            for (i, provider) in embed_config.providers.iter().enumerate() {
                providers_table
                    .set(i + 1, provider.as_str())
                    .map_err(|e| format!("Failed to set provider: {}", e))?;
            }
            params
                .set("providers", providers_table)
                .map_err(|e| format!("Failed to set providers: {}", e))?;

            // Convert models HashMap to Lua table
            let models_table = lua
                .create_table()
                .map_err(|e| format!("Failed to create models table: {}", e))?;
            for (provider, model_list) in &embed_config.models {
                let model_array = lua
                    .create_table()
                    .map_err(|e| format!("Failed to create model array: {}", e))?;
                for (i, model) in model_list.iter().enumerate() {
                    model_array
                        .set(i + 1, model.as_str())
                        .map_err(|e| format!("Failed to set model: {}", e))?;
                }
                models_table
                    .set(provider.as_str(), model_array)
                    .map_err(|e| format!("Failed to set model list: {}", e))?;
            }
            params
                .set("models", models_table)
                .map_err(|e| format!("Failed to set models: {}", e))?;

            let cap = runtime
                .create_capability("embed", params)
                .map_err(|e| format!("Failed to create embed capability '{}': {}", name, e))?;

            embed_caps
                .set(name.as_str(), cap)
                .map_err(|e| format!("Failed to set capability: {}", e))?;
        }

        caps.set("embed", embed_caps)
            .map_err(|e| format!("Failed to set embed caps: {}", e))?;
    }

    // Create libsql capabilities
    if !caps_config.libsql.is_empty() {
        let libsql_caps = lua
            .create_table()
            .map_err(|e| format!("Failed to create libsql caps table: {}", e))?;

        for (name, libsql_config) in &caps_config.libsql {
            let params = lua
                .create_table()
                .map_err(|e| format!("Failed to create params: {}", e))?;

            if let Some(ref path) = libsql_config.path {
                let expanded_path = expand_path(path, project_path);
                params
                    .set("path", expanded_path)
                    .map_err(|e| format!("Failed to set path: {}", e))?;
            }
            params
                .set("allow_memory", libsql_config.allow_memory)
                .map_err(|e| format!("Failed to set allow_memory: {}", e))?;

            let cap = runtime
                .create_capability("libsql", params)
                .map_err(|e| format!("Failed to create libsql capability '{}': {}", name, e))?;

            libsql_caps
                .set(name.as_str(), cap)
                .map_err(|e| format!("Failed to set capability: {}", e))?;
        }

        caps.set("libsql", libsql_caps)
            .map_err(|e| format!("Failed to set libsql caps: {}", e))?;
    }

    Ok(caps)
}

/// Expand variables in a path string.
fn expand_path(path: &str, project_path: &Path) -> String {
    path.replace("${PROJECT_ROOT}", &project_path.to_string_lossy())
        .replace("$PROJECT_ROOT", &project_path.to_string_lossy())
}

fn init_project(path: &Path) -> Result<(), String> {
    let spore_dir = path.join(".spore");

    if spore_dir.exists() {
        return Err("Project already initialized (.spore directory exists)".to_string());
    }

    std::fs::create_dir_all(&spore_dir)
        .map_err(|e| format!("Failed to create .spore directory: {}", e))?;

    let config_content = r#"# spore configuration

[project]
entry = "main.lua"

[plugins]
fs = false
llm = false
embed = false
libsql = false
moss = false
sessions = false
tools = false
packages = false

# Sandbox configuration for require()
# Builtins always take precedence and cannot be overridden by user code
[sandbox]
require_builtins = true   # Allow require("string"), require("table"), etc.
require_plugins = true    # Allow require("spore.sessions"), require("spore.llm"), etc.
require_project = true    # Allow require("mymodule") from project directory

# Capability configuration
# Capabilities are created from plugin parameters and injected into scripts
# Scripts access them via caps.{plugin}.{name}, e.g., caps.fs.project

# [caps.fs]
# project = { path = "${PROJECT_ROOT}", mode = "rw" }
# tmp = { path = "/tmp", mode = "rw" }

# [caps.moss]
# project = { root = "${PROJECT_ROOT}", mode = "rw" }

# [caps.sessions]
# project = { root = "${PROJECT_ROOT}" }

# [caps.tools]
# project = { root = "${PROJECT_ROOT}" }

# [caps.packages]
# project = { root = "${PROJECT_ROOT}" }

# [caps.llm]
# project = { providers = ["anthropic", "openai"] }
# budget = { providers = ["anthropic"], models = { anthropic = ["claude-haiku-3.5"] } }

# [caps.embed]
# project = { providers = ["openai", "cohere"] }

# [caps.libsql]
# data = { path = "${PROJECT_ROOT}/.spore/data" }
# scratch = { allow_memory = true }
# full = { path = "${PROJECT_ROOT}/data", allow_memory = true }
"#;

    std::fs::write(spore_dir.join("config.toml"), config_content)
        .map_err(|e| format!("Failed to write config: {}", e))?;

    println!("Initialized spore project in {}", path.display());
    println!("  .spore/config.toml - configuration");

    Ok(())
}
