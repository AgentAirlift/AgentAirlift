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
    },
}