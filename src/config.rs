use anyhow::Result;
use jsonc_parser::parse_to_serde_value;
use serde::Deserialize;
use std::fs;

fn default_model_name() -> String {
    "gpt-5-mini".to_string()
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

#[derive(Deserialize, Debug)]
pub struct Config {
    pub trae_executable_path: String,
    #[serde(default = "default_command_strategy")]
    pub command_strategy: CommandStrategy,
    #[serde(default = "default_question_strategy")]
    pub question_strategy: QuestionStrategy,
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default = "default_max_concurrent_task")]
    pub max_concurrent_task: u32,
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let raw_json_data = fs::read_to_string("config.jsonc")
            .expect("Cannot read configuration file `config.jsonc` in current working directory.");

        let config: Config = parse_to_serde_value(&raw_json_data, &Default::default())?;
        Ok(config)
    }
}
