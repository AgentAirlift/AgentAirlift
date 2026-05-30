use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "agent-airlift")]
#[command(about = "Agent Airlift: Migrate AI sessions between providers")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run a demo migration with local files
    Demo {
        /// Path to session JSONL file
        #[arg(long)]
        session: String,
        
        /// Path to project directory
        #[arg(long)]
        project: String,
        
        /// Output directory
        #[arg(long)]
        out: String,
        
        /// Source provider (e.g., claude-code)
        #[arg(long)]
        source: String,
        
        /// Target providers (comma-separated)
        #[arg(long, value_delimiter = ',')]
        targets: Vec<String>,
        
        /// Provider health source type
        #[arg(long, default_value = "none")]
        provider_health: String,
        
        /// Path to provider health file (if using file source)
        #[arg(long)]
        provider_health_file: Option<String>,

        /// Upload output to Box
        #[arg(long, default_value_t = false)]
        box_upload: bool,

        /// Dry-run Box upload (print what would be uploaded, no API calls)
        #[arg(long, default_value_t = false)]
        box_dry_run: bool,

        /// Box parent folder ID (overrides BOX_PARENT_FOLDER_ID env var)
        #[arg(long)]
        box_parent_folder_id: Option<String>,
    },
}