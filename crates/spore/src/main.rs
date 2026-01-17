//! spore CLI - Run Lua scripts with spore integrations.

use clap::{Parser, Subcommand};
use rhizome_spore_llm::LlmIntegration;
use rhizome_spore_lua::Runtime;
use rhizome_spore_moss::MossIntegration;
use rhizome_spore_moss_packages::MossPackagesIntegration;
use rhizome_spore_moss_sessions::MossSessionsIntegration;
use rhizome_spore_moss_tools::MossToolsIntegration;
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
    integrations: IntegrationsConfig,
    #[serde(default)]
    plugins: PluginsConfig,
    #[serde(default)]
    caps: CapsConfig,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ProjectConfig {
    entry: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct IntegrationsConfig {
    #[serde(default)]
    llm: bool,
    #[serde(default)]
    moss: bool,
    #[serde(default)]
    moss_sessions: bool,
    #[serde(default)]
    moss_tools: bool,
    #[serde(default)]
    moss_packages: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct PluginsConfig {
    #[serde(default)]
    fs: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
struct CapsConfig {
    #[serde(default)]
    fs: std::collections::HashMap<String, FsCapConfig>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct FsCapConfig {
    path: String,
    #[serde(default = "default_mode")]
    mode: String,
}

fn default_mode() -> String {
    "r".to_string()
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
        Commands::Run { path, entry } => {
            if let Err(e) = run(&path, entry.as_deref()) {
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

fn run(project_path: &Path, entry_override: Option<&Path>) -> Result<(), String> {
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

    // Register integrations based on config
    if config.integrations.llm {
        runtime
            .register(&LlmIntegration::new())
            .map_err(|e| format!("Failed to register llm integration: {}", e))?;
    }

    if config.integrations.moss {
        runtime
            .register(&MossIntegration::new(&project_path))
            .map_err(|e| format!("Failed to register moss integration: {}", e))?;
    }

    if config.integrations.moss_sessions {
        runtime
            .register(&MossSessionsIntegration)
            .map_err(|e| format!("Failed to register moss_sessions integration: {}", e))?;
    }

    if config.integrations.moss_tools {
        runtime
            .register(&MossToolsIntegration::new(&project_path))
            .map_err(|e| format!("Failed to register moss_tools integration: {}", e))?;
    }

    if config.integrations.moss_packages {
        runtime
            .register(&MossPackagesIntegration::new(&project_path))
            .map_err(|e| format!("Failed to register moss_packages integration: {}", e))?;
    }

    // Load plugins based on config
    if config.plugins.fs {
        runtime
            .load_plugin("fs")
            .map_err(|e| format!("Failed to load fs plugin: {}", e))?;
    }

    // Set project root in Lua
    runtime
        .lua()
        .globals()
        .get::<mlua::Table>("spore")
        .and_then(|spore| spore.set("root", project_path.to_string_lossy().to_string()))
        .map_err(|e| format!("Failed to set project root: {}", e))?;

    // Create capabilities from config
    let caps = create_capabilities(&runtime, &config.caps, &project_path)?;

    // Run the entry point with capabilities
    let code = std::fs::read_to_string(&entry)
        .map_err(|e| format!("Failed to read entry script: {}", e))?;

    runtime
        .run_with_caps(&code, caps)
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
            // Expand variables in path
            let expanded_path = expand_path(&fs_config.path, project_path);

            // Create params table
            let params = lua
                .create_table()
                .map_err(|e| format!("Failed to create params: {}", e))?;
            params
                .set("path", expanded_path)
                .map_err(|e| format!("Failed to set path: {}", e))?;
            params
                .set("mode", fs_config.mode.clone())
                .map_err(|e| format!("Failed to set mode: {}", e))?;

            // Create capability
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

[integrations]
llm = false
moss = false
moss_sessions = false
moss_tools = false
moss_packages = false

[plugins]
fs = false

# Capability configuration
# Capabilities are created from plugin parameters and injected into scripts
# Scripts access them via caps.{plugin}.{name}, e.g., caps.fs.project

# [caps.fs]
# project = { path = "${PROJECT_ROOT}", mode = "rw" }
# tmp = { path = "/tmp", mode = "rw" }
"#;

    std::fs::write(spore_dir.join("config.toml"), config_content)
        .map_err(|e| format!("Failed to write config: {}", e))?;

    println!("Initialized spore project in {}", path.display());
    println!("  .spore/config.toml - configuration");

    Ok(())
}
