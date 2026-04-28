use crate::cli::{Cli, Commands, ConfigCommands, Runner};
use crate::runner::run_trae;
use clap::Parser;
use gothic::config::{self, validate_config};

mod cli;
mod runner;

type AppResult<T> = Result<T, Box<dyn std::error::Error>>;

#[tokio::main]
async fn main() -> AppResult<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { app, tasks } => run(app, tasks).await?,
        Commands::Config { command } => handle_config(command)?,
    }

    Ok(())
}

async fn run(app: Runner, tasks: Vec<String>) -> AppResult<()> {
    match app {
        Runner::Trae => run_trae(tasks).await?,
    }

    Ok(())
}

fn handle_config(command: ConfigCommands) -> AppResult<()> {
    match command {
        ConfigCommands::Init { path, force } => {
            let path = config::init_config(path, force)?;
            println!("created config: {}", path.display());
        }
        ConfigCommands::Check { path } => {
            let path = config::resolve_config_path(path)?;
            let config = config::load_config(&path)?;
            validate_config(&config)?;
        }
    }

    Ok(())
}
