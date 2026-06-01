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
    /// Run the migration pipeline (alias: demo)
    #[command(visible_alias = "demo")]
    Migrate {
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

        /// Apify actor ID (overrides APIFY_ACTOR_ID env var)
        #[arg(long)]
        apify_actor_id: Option<String>,

        /// Apify task ID (overrides APIFY_TASK_ID env var)
        #[arg(long)]
        apify_task_id: Option<String>,

        /// URL to pass as input to the Apify actor/task
        #[arg(long)]
        apify_input_url: Option<String>,

        /// Fallback cache file if Apify live call fails
        #[arg(long)]
        apify_cache_file: Option<String>,

        /// Skip installing native resume-compatible sessions into ~/.codex / ~/.claude
        #[arg(long, default_value_t = false)]
        skip_native_install: bool,

        /// Override the agent home root for native installs (tests / non-default $HOME)
        #[arg(long)]
        native_home: Option<String>,
    },

    /// Refresh the provider-health signal only (no migration)
    Health {
        /// Provider being evaluated (e.g., claude-code)
        #[arg(long)]
        source: String,

        /// Output directory; persists <out>/provider-health.json
        #[arg(long, default_value = "./airlift-out")]
        out: String,

        /// Provider health source type
        #[arg(long, default_value = "apify")]
        provider_health: String,

        /// Path to provider health file (if using file source)
        #[arg(long)]
        provider_health_file: Option<String>,

        /// Apify actor ID (overrides APIFY_ACTOR_ID env var)
        #[arg(long)]
        apify_actor_id: Option<String>,

        /// Apify task ID (overrides APIFY_TASK_ID env var)
        #[arg(long)]
        apify_task_id: Option<String>,

        /// URL to pass as input to the Apify actor/task
        #[arg(long)]
        apify_input_url: Option<String>,

        /// Fallback cache file if Apify live call fails
        #[arg(long)]
        apify_cache_file: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::{CommandFactory, Parser};

    #[test]
    fn migrate_rejects_box_upload_flags() {
        let result = Cli::try_parse_from([
            "agent-airlift",
            "migrate",
            "--session",
            "examples/sessions/claude-code-realistic.jsonl",
            "--project",
            "examples/projects/tiny-rust-cli",
            "--out",
            "airlift-out",
            "--source",
            "claude-code",
            "--targets",
            "codex",
            "--box-upload",
        ]);

        assert!(result.is_err(), "Box upload flags should not be accepted");
    }

    #[test]
    fn migrate_help_does_not_advertise_box_upload() {
        let mut help = Vec::new();
        Cli::command()
            .find_subcommand_mut("migrate")
            .expect("migrate subcommand exists")
            .write_long_help(&mut help)
            .expect("help renders");
        let help = String::from_utf8(help).expect("help is utf8");

        assert!(!help.contains("box-upload"));
        assert!(!help.contains("box-dry-run"));
        assert!(!help.contains("box-parent-folder-id"));
    }
}
