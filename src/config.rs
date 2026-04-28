use anyhow::{Context, Result, anyhow, bail};
use jsonc_parser::parse_to_serde_value;
use serde::Deserialize;
use std::{
    env, fs,
    path::{Path, PathBuf},
};
use tracing_subscriber::EnvFilter;

pub const DEFAULT_CONFIG_JSONC: &str = include_str!("../config.example.jsonc");

fn default_model_name() -> String {
    "gpt-5-mini".to_string()
}

fn default_log_directory() -> String {
    "logs".to_string()
}

fn default_log_level() -> String {
    "info".to_string()
}

#[derive(Deserialize, Debug, Clone)]
pub struct ModelConfig {
    pub api_key: String,
    pub base_url: String,
    #[serde(default = "default_model_name")]
    pub model_name: String,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: String::new(),
            model_name: default_model_name(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
pub struct LoggingConfig {
    #[serde(default = "default_log_directory")]
    pub directory: String,
    #[serde(default = "default_log_level")]
    pub level: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            directory: default_log_directory(),
            level: default_log_level(),
        }
    }
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum CommandStrategy {
    Allow,
    Deny,
    LLM,
}

#[derive(Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum QuestionStrategy {
    Skip,
    Auto,
    LLM,
}

fn default_command_strategy() -> CommandStrategy {
    CommandStrategy::Allow
}

fn default_question_strategy() -> QuestionStrategy {
    QuestionStrategy::Auto
}

fn default_max_concurrent_task() -> u32 {
    5
}

fn default_max_task_action_retry() -> u32 {
    3
}

fn default_task_poll_interval_ms() -> u64 {
    2000
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub trae_executable_path: String,
    #[serde(default = "default_command_strategy")]
    pub command_strategy: CommandStrategy,
    #[serde(default = "default_question_strategy")]
    pub question_strategy: QuestionStrategy,
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default = "default_max_concurrent_task")]
    pub max_concurrent_task: u32,
    /// Retry actionable UI flows such as Interrupted/WaitingForHITL a few times
    /// before giving up and warning in the console.
    #[serde(default = "default_max_task_action_retry")]
    pub max_task_action_retry: u32,
    /// Poll interval for syncing Trae sidebar task updates.
    #[serde(default = "default_task_poll_interval_ms")]
    pub task_poll_interval_ms: u64,
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let raw_json_data = fs::read_to_string("config.jsonc")
            .expect("Cannot read configuration file `config.jsonc` in current working directory.");

        let config: Config = parse_to_serde_value(&raw_json_data, &Default::default())?;
        validate_config(&config)?;
        Ok(config)
    }
}

pub fn resolve_config_path(path: Option<PathBuf>) -> Result<PathBuf> {
    match path {
        Some(path) => Ok(path),
        None => {
            let exe_path = env::current_exe().context("failed to get current executable path")?;

            let exe_dir = exe_path
                .parent()
                .context("failed to get executable parent directory")?;

            Ok(exe_dir.join("config.jsonc"))
        }
    }
}

pub fn load_config(path: &Path) -> Result<Config> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;

    let config: Config = parse_to_serde_value(&text, &Default::default())
        .map_err(|err| anyhow!("invalid JSONC config at {}: {err}", path.display()))?;

    validate_config(&config)?;

    Ok(config)
}

pub fn init_config(path: Option<PathBuf>, force: bool) -> Result<PathBuf> {
    let path = resolve_config_path(path)?;

    if path.exists() && !force {
        bail!(
            "config file already exists: {}\nuse --force to overwrite it",
            path.display()
        );
    }

    // write default config file
    fs::write(&path, DEFAULT_CONFIG_JSONC)
        .with_context(|| format!("failed to write config file: {}", path.display()))?;

    Ok(path)
}

pub fn validate_config(config: &Config) -> Result<()> {
    let mut errors = Vec::new();

    if config.trae_executable_path.trim().is_empty() {
        errors.push("trae_executable_path cannot be empty".to_string());
    }

    if config.max_concurrent_task < 1 {
        errors.push("max_concurrent_task must be greater than or equal to 1".to_string());
    }

    if config.max_task_action_retry < 1 {
        errors.push("max_task_action_retry must be greater than or equal to 1".to_string());
    }

    if config.task_poll_interval_ms < 100 {
        errors.push("task_poll_interval_ms must be greater than or equal to 100".to_string());
    }

    if config.logging.directory.trim().is_empty() {
        errors.push("logging.directory cannot be empty".to_string());
    }

    let log_level = config.logging.level.trim();
    if log_level.is_empty() {
        errors.push("logging.level cannot be empty".to_string());
    } else if let Err(err) = EnvFilter::try_new(log_level) {
        errors.push(format!(
            "logging.level must be a valid tracing filter: {err}"
        ));
    }

    let uses_llm = matches!(config.command_strategy, CommandStrategy::LLM)
        || matches!(config.question_strategy, QuestionStrategy::LLM);
    if uses_llm && config.model.model_name.trim().is_empty() {
        errors.push(
            "model.model_name cannot be empty when command_strategy or question_strategy is \"llm\""
                .to_string(),
        );
    }

    if !errors.is_empty() {
        bail!("invalid config:\n- {}", errors.join("\n- "));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_config() -> Config {
        Config {
            trae_executable_path: r"C:\Program Files\Trae\Trae.exe".to_string(),
            command_strategy: CommandStrategy::Allow,
            question_strategy: QuestionStrategy::Auto,
            model: ModelConfig::default(),
            logging: LoggingConfig::default(),
            max_concurrent_task: default_max_concurrent_task(),
            max_task_action_retry: default_max_task_action_retry(),
            task_poll_interval_ms: default_task_poll_interval_ms(),
        }
    }

    #[test]
    fn accepts_default_non_llm_model_settings() {
        let config = sample_config();

        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn rejects_empty_required_values() {
        let mut config = sample_config();
        config.trae_executable_path.clear();
        config.logging.directory.clear();

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("trae_executable_path cannot be empty"));
        assert!(err.contains("logging.directory cannot be empty"));
    }

    #[test]
    fn rejects_schema_minimum_violations() {
        let mut config = sample_config();
        config.max_concurrent_task = 0;
        config.max_task_action_retry = 0;
        config.task_poll_interval_ms = 99;

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("max_concurrent_task must be greater than or equal to 1"));
        assert!(err.contains("max_task_action_retry must be greater than or equal to 1"));
        assert!(err.contains("task_poll_interval_ms must be greater than or equal to 100"));
    }

    #[test]
    fn rejects_invalid_log_level() {
        let mut config = sample_config();
        config.logging.level = "[".to_string();

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains("logging.level must be a valid tracing filter"));
    }

    #[test]
    fn rejects_empty_model_name_for_llm_strategies() {
        let mut config = sample_config();
        config.question_strategy = QuestionStrategy::LLM;
        config.model.model_name.clear();

        let err = validate_config(&config).unwrap_err().to_string();

        assert!(err.contains(
            "model.model_name cannot be empty when command_strategy or question_strategy is \"llm\""
        ));
    }
}
