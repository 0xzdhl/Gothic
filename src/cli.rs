use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "gothic",
    version,
    about = "A CLI automation tool for orchestrating other agentic coding IDE."
)]
pub struct Cli {
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    Run {
        #[arg(value_enum)]
        app: Runner,

        #[arg(long = "task", value_name = "TASK", action = ArgAction::Append)]
        tasks: Vec<String>,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Runner {
    Trae,
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
    /// Generate the default config.jsonc
    Init {
        /// Custom config file path
        #[arg(short, long)]
        path: Option<PathBuf>,

        /// Overwrite existing config file
        #[arg(short, long)]
        force: bool,
    },

    /// Check and validate config.jsonc
    Check {
        /// Custom config file path
        #[arg(short, long)]
        path: Option<PathBuf>,
    },
}
