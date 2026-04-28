use crate::cli::{Cli, Commands, ConfigCommands, Runner};
use chromiumoxide::Browser;
use clap::Parser;
use futures::StreamExt;
use gothic::config::{self, Config, validate_config};
use gothic::logging::init_logging;
use gothic::trae::{ActionChain, InitialTaskPolicy, TaskWorkflow, TraeEditor, TraeEditorMode};
use gothic::utils::{wait_for_debug_port, wait_for_shutdown};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, instrument};

mod cli;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Run { app, tasks } => run(app, tasks).await?,
        Commands::Config { command } => handle_config(command)?,
    }

    Ok(())
}

async fn run(app: Runner, tasks: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    match app {
        Runner::Trae => {
            let config = Config::load()?;
            let _logging = init_logging(&config.logging)?;

            info!("Logging initialized");

            let mut trae_main = Command::new(&config.trae_executable_path)
                .arg("--remote-debugging-port=9222")
                .arg("--no-sandbox")
                .stdout(Stdio::null()) // inherit current stream
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()?;

            // let trae_pid = trae_main.id().expect("Cannot get Trae PID.");

            wait_for_debug_port(9222, Duration::from_secs(30)).await?;

            // connect to CDP
            let (mut browser, mut handler) = Browser::connect("http://127.0.0.1:9222").await?;
            info!("Successfully connected to Trae via CDP: 127.0.0.1:9222");

            // spawn a new task that continuously polls the handler
            let handle = tokio::spawn(async move {
                while let Some(event) = handler.next().await {
                    match event {
                        Ok(_) => {}
                        Err(e) => {
                            error!("Browser event handler failed: {e}");
                            break;
                        }
                    }
                }
            });

            let trae_editor_builder = TraeEditor::new();

            let mut trae_editor = trae_editor_builder.build(&mut browser, config).await;

            let (shutdown_tx, shutdown_rx) = watch::channel(false);

            info!("Current Trae mode: {:?}", trae_editor.mode);

            // switch mode
            trae_editor.switch_editor_mode(TraeEditorMode::SOLO).await?;

            // create tasks
            for task in tasks {
                // create a new task
                quick_task(&task, &trae_editor).await;
            }

            sleep(Duration::from_millis(3000)).await;

            // get tasks from panel
            let task_poll_interval =
                Duration::from_millis(trae_editor.config.task_poll_interval_ms);
            let arc_editor = Arc::new(trae_editor);
            let arc_editor_for_loop = Arc::clone(&arc_editor);

            let workflow = build_task_workflow();

            // launch event loop
            tokio::spawn(async move {
                arc_editor_for_loop
                    .run_task_sync_loop(
                        task_poll_interval,
                        workflow,
                        InitialTaskPolicy::EmitTerminalAndWaiting,
                        shutdown_rx,
                    )
                    .await;
            });

            // sleep 3 secs
            sleep(Duration::from_millis(3000)).await;

            let tasks = arc_editor.cached_tasks().await;

            debug!(tasks = ?tasks, "Cached tasks snapshot");

            // receive ctrl+c signal
            wait_for_shutdown().await?;

            // stop fetching
            let _ = shutdown_tx.send(true);

            // close browser
            let _ = browser.close().await?;

            // join await
            let _ = handle.await?;

            let _ = trae_main.wait().await?;
        }
    }

    Ok(())
}

fn handle_config(command: ConfigCommands) -> Result<(), Box<dyn std::error::Error>> {
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

#[instrument(skip(editor))]
async fn quick_task(prompt: &str, editor: &TraeEditor) {
    let task = editor.create_new_task(prompt.to_string()).await;

    // execute task
    match task.execute().await {
        Ok(_) => info!("Task executed successfully. ({})", prompt),
        Err(e) => error!("Task execution failed. ({}): {e}", prompt),
    }

    // sleep 1 sec
    sleep(Duration::from_millis(3000)).await;
}

fn build_task_workflow() -> TaskWorkflow {
    TaskWorkflow {
        on_finished: ActionChain::new()
            .new_task()
            .focus_chat_input()
            .clear_chat_input()
            .type_text("Create new task")
            .sleep_ms(1000)
            .press_enter(),

        on_interrupted: ActionChain::new()
            .focus_task()
            .focus_chat_input()
            .clear_chat_input()
            .type_text("继续")
            .sleep_ms(1000)
            .press_enter(),

        on_waiting_for_hitl: ActionChain::new()
            .focus_task()
            .sleep_ms(1000)
            .handle_human_in_loop(),
    }
}
