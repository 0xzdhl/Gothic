use chromiumoxide::Browser;
use futures::StreamExt;
use gothic::trae::{ActionChain, InitialTaskPolicy, TaskWorkflow, TraeEditor, TraeEditorMode};
use gothic::utils::wait_for_debug_port;
use gothic::{config::Config, logging::init_logging, utils::wait_for_shutdown};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::{Child, Command};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::{Duration, sleep, timeout};
use tracing::{debug, error, info, instrument};

use crate::AppResult;

fn shutdown_requested(shutdown_rx: &watch::Receiver<bool>) -> bool {
    *shutdown_rx.borrow()
}

async fn shutdown_child(mut child: Child) {
    match timeout(Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => debug!(?status, "Trae process exited"),
        Ok(Err(err)) => error!("Failed waiting for Trae process: {err}"),
        Err(_) => {
            info!("Trae process did not exit in time, terminating it");

            if let Err(err) = child.kill().await {
                error!("Failed to terminate Trae process: {err}");
                return;
            }

            match child.wait().await {
                Ok(status) => debug!(?status, "Trae process terminated"),
                Err(err) => error!("Failed waiting for terminated Trae process: {err}"),
            }
        }
    }
}

async fn wait_for_shutdown_request(shutdown_rx: &mut watch::Receiver<bool>) {
    if shutdown_requested(shutdown_rx) {
        return;
    }

    let _ = shutdown_rx.changed().await;
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

pub async fn run_trae(tasks: Vec<String>) -> AppResult<()> {
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let shutdown_listener = tokio::spawn({
        let shutdown_tx = shutdown_tx.clone();
        async move {
            match wait_for_shutdown().await {
                Ok(()) => {
                    info!("Received Ctrl+C, starting graceful shutdown");
                    let _ = shutdown_tx.send(true);
                }
                Err(err) => error!("Failed to listen for Ctrl+C: {err}"),
            }
        }
    });
    let mut main_shutdown_rx = shutdown_rx.clone();

    let config = Config::load()?;
    let _logging = init_logging(&config.logging)?;

    info!("Logging initialized");

    let mut trae_main = Some(
        Command::new(&config.trae_executable_path)
            .arg("--remote-debugging-port=9222")
            .arg("--no-sandbox")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()?,
    );

    let mut browser = None;
    let mut browser_handler = None;
    let mut sync_loop = None;

    let run_result: AppResult<()> = async {
        tokio::select! {
            wait_result = wait_for_debug_port(9222, Duration::from_secs(30)) => wait_result?,
            _ = wait_for_shutdown_request(&mut main_shutdown_rx) => return Ok(()),
        }

        let (mut connected_browser, mut handler) =
            Browser::connect("http://127.0.0.1:9222").await?;
        info!("Successfully connected to Trae via CDP: 127.0.0.1:9222");

        browser_handler = Some(tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                match event {
                    Ok(_) => {}
                    Err(e) => {
                        error!("Browser event handler failed: {e}");
                        break;
                    }
                }
            }
        }));

        let trae_editor_builder = TraeEditor::new();
        let mut trae_editor = trae_editor_builder
            .build(&mut connected_browser, config)
            .await;
        browser = Some(connected_browser);

        if shutdown_requested(&main_shutdown_rx) {
            return Ok(());
        }

        info!("Current Trae mode: {:?}", trae_editor.mode);

        trae_editor.switch_editor_mode(TraeEditorMode::SOLO).await?;

        if shutdown_requested(&main_shutdown_rx) {
            return Ok(());
        }

        for task in tasks {
            quick_task(&task, &trae_editor).await;

            if shutdown_requested(&main_shutdown_rx) {
                return Ok(());
            }
        }

        sleep(Duration::from_millis(3000)).await;

        if shutdown_requested(&main_shutdown_rx) {
            return Ok(());
        }

        let task_poll_interval = Duration::from_millis(trae_editor.config.task_poll_interval_ms);
        let arc_editor = Arc::new(trae_editor);
        let arc_editor_for_loop = Arc::clone(&arc_editor);
        let workflow = build_task_workflow();
        let task_sync_shutdown_rx = shutdown_rx.clone();

        sync_loop = Some(tokio::spawn(async move {
            arc_editor_for_loop
                .run_task_sync_loop(
                    task_poll_interval,
                    workflow,
                    InitialTaskPolicy::EmitTerminalAndWaiting,
                    task_sync_shutdown_rx,
                )
                .await;
        }));

        sleep(Duration::from_millis(3000)).await;

        let tasks = arc_editor.cached_tasks().await;
        debug!(tasks = ?tasks, "Cached tasks snapshot");

        wait_for_shutdown_request(&mut main_shutdown_rx).await;
        Ok(())
    }
    .await;

    cleanup_trae_run(
        &shutdown_tx,
        sync_loop,
        browser,
        browser_handler,
        trae_main.take(),
    )
    .await;

    if !shutdown_listener.is_finished() {
        shutdown_listener.abort();
    }
    let _ = shutdown_listener.await;

    run_result
}

async fn cleanup_trae_run(
    shutdown_tx: &watch::Sender<bool>,
    sync_loop: Option<JoinHandle<()>>,
    browser: Option<Browser>,
    browser_handler: Option<JoinHandle<()>>,
    trae_main: Option<Child>,
) {
    let _ = shutdown_tx.send(true);

    if let Some(sync_loop) = sync_loop {
        if let Err(err) = sync_loop.await {
            error!("Task sync loop join failed: {err}");
        }
    }

    if let Some(mut browser) = browser {
        if let Err(err) = browser.close().await {
            error!("Failed to close browser cleanly: {err}");
        }
    }

    if let Some(browser_handler) = browser_handler {
        if let Err(err) = browser_handler.await {
            error!("Browser event handler join failed: {err}");
        }
    }

    if let Some(trae_main) = trae_main {
        shutdown_child(trae_main).await;
    }
}
