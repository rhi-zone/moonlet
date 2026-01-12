//! spore CLI - Run Lua scripts with spore integrations.

use clap::{Parser, Subcommand};
use rhizome_spore_llm::LlmIntegration;
use rhizome_spore_lua::Runtime;
use rhizome_spore_moss::MossIntegration;
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

#[derive(Debug, Deserialize)]
struct Config {
    project: ProjectConfig,
    #[serde(default)]
    integrations: IntegrationsConfig,
}

#[derive(Debug, Deserialize)]
struct ProjectConfig {
    entry: String,
}

#[derive(Debug, Default, Deserialize)]
struct IntegrationsConfig {
    #[serde(default)]
    llm: bool,
    #[serde(default)]
    moss: bool,
}

fn main() {
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
    let runtime = Runtime::new().map_err(|e| format!("Failed to create runtime: {}", e))?;

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

    // Set project root in Lua
    runtime
        .lua()
        .globals()
        .get::<mlua::Table>("spore")
        .and_then(|spore| spore.set("root", project_path.to_string_lossy().to_string()))
        .map_err(|e| format!("Failed to set project root: {}", e))?;

    // Run the entry point
    runtime
        .run_file(&entry)
        .map_err(|e| format!("Script error: {}", e))?;

    Ok(())
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
"#;

    std::fs::write(spore_dir.join("config.toml"), config_content)
        .map_err(|e| format!("Failed to write config: {}", e))?;

    println!("Initialized spore project in {}", path.display());
    println!("  .spore/config.toml - configuration");

    Ok(())
}
