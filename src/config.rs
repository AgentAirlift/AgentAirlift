use std::path::PathBuf;
use chrono::{DateTime, Utc};

#[derive(Debug)]
pub struct Config {
    pub session_path: PathBuf,
    pub project_path: PathBuf,
    pub output_dir: PathBuf,
    pub source_provider: String,
    pub target_providers: Vec<String>,
    pub provider_health_source: String,
    pub provider_health_file: Option<PathBuf>,
    #[allow(dead_code)]
    pub timestamp: DateTime<Utc>,
}

impl Config {
    pub fn from_cli(session: String, project: String, out: String, source: String, 
                   targets: Vec<String>, provider_health: String, 
                   provider_health_file: Option<String>) -> anyhow::Result<Self> {
        let timestamp = Utc::now();
        
        Ok(Self {
            session_path: PathBuf::from(session),
            project_path: PathBuf::from(project),
            output_dir: PathBuf::from(out),
            source_provider: source,
            target_providers: targets,
            provider_health_source: provider_health,
            provider_health_file: provider_health_file.map(PathBuf::from),
            timestamp,
        })
    }
    
    pub fn create_output_dirs(&self) -> anyhow::Result<()> {
        let dirs = vec![
            self.output_dir.join("raw"),
            self.output_dir.join("normalized"),
            self.output_dir.join("replay"),
            self.output_dir.join("exports"),
            self.output_dir.join("exports/.kiro/specs/agent-airlift-handoff"),
            self.output_dir.join("audit"),
        ];
        
        for dir in dirs {
            std::fs::create_dir_all(&dir)?;
        }
        
        Ok(())
    }
}