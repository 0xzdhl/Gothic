use anyhow::Result;
use jsonc_parser::parse_to_serde_value;
use serde::Deserialize;
use std::fs;

#[derive(Deserialize, Debug)]
pub struct ModelConfig {
    #[allow(dead_code)]
    api_key: String,
    #[allow(dead_code)]
    base_url: String,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum CommandStrategy {
    Allow,
    Deny,
    LLM,
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum QuestionStrategy {
    Skip,
    Auto,
    LLM,
}

#[derive(Deserialize, Debug)]
pub struct Config {
    pub trae_executable_path: String,
    pub command_strategy: CommandStrategy,
    pub question_strategy: QuestionStrategy,
    pub model: ModelConfig,
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let raw_json_data = fs::read_to_string("config.jsonc")
            .expect("Cannot read configuration file `config.jsonc` in current working directory.");

        let config: Config = parse_to_serde_value(&raw_json_data, &Default::default())?;
        Ok(config)
    }
}
