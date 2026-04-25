use crate::config::LoggingConfig;
use anyhow::Result;
use tracing_appender::{
    non_blocking::{NonBlockingBuilder, WorkerGuard},
    rolling,
};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

const LOG_FILE_NAME: &str = "gothic.log";

pub struct LoggingGuards {
    // Keep the guards alive until process shutdown so buffered logs are flushed.
    _console_guard: WorkerGuard,
    _file_guard: WorkerGuard,
}

pub fn init_logging(config: &LoggingConfig) -> Result<LoggingGuards> {
    std::fs::create_dir_all(&config.directory)?;

    let (console_writer, console_guard) = NonBlockingBuilder::default()
        .lossy(false)
        .finish(std::io::stderr());

    let file_appender = rolling::daily(&config.directory, LOG_FILE_NAME);
    let (file_writer, file_guard) = NonBlockingBuilder::default()
        .lossy(false)
        .finish(file_appender);

    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(&config.level))
        .expect("configured log filter must be valid");

    let console_layer = fmt::layer()
        .with_ansi(true)
        .with_target(false)
        .with_writer(console_writer);

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_thread_names(true)
        .with_writer(file_writer);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(console_layer)
        .with(file_layer)
        .try_init()?;

    Ok(LoggingGuards {
        _console_guard: console_guard,
        _file_guard: file_guard,
    })
}
