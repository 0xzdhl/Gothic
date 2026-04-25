use chromiumoxide::Browser;
use futures::StreamExt;
use gothic::config::Config;
use gothic::logging::init_logging;
use gothic::trae::{
    ActionChain, CustomActionExample, InitialTaskPolicy, TaskWorkflow, TraeEditor, TraeEditorMode,
};
use gothic::utils::{wait_for_debug_port, wait_for_shutdown};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// Load config and logging
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

    // create a new task
    {
        quick_task("翻译：我喜欢使用Python编程为日语。", &trae_editor).await;
        quick_task("帮我制作一个网站，制作之前你要先提供一些选项给我让我选择（单选+多选）要做什么类型的网站，我选完之后你再开始写", &trae_editor).await;
    }

    // sleep 3 secs
    sleep(Duration::from_millis(3000)).await;
    // get tasks from panel
    let arc_editor = Arc::new(trae_editor);
    let arc_editor_for_loop = Arc::clone(&arc_editor);

    let workflow = build_task_workflow();

    tokio::spawn(async move {
        arc_editor_for_loop
            .run_task_sync_loop(
                Duration::from_secs(2),
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

    // click the second task

    if tasks.len() > 3 {
        let second_task = tasks.get(1).unwrap();
        let second_task_handler = arc_editor
            .get_task_handle_by_id(second_task.task_id)
            .await?;

        // trigger selection
        second_task_handler.select().await?;

        // try type something in it
        second_task_handler.type_content("fuck everything.").await?;

        // switch to third item, copy summary text

        // let third_task = tasks.get(2).unwrap();
        // let third_task_handler = arc_editor
        //     .get_task_handle_by_id(third_task.task_id)
        //     .await?;

        // let text_summary = third_task_handler.copy_summary().await?;
        // third_task_handler.type_content("test content").await?;

        // println!("The text summary of the third task: {}", text_summary);
    }

    // receive ctrl+c signal
    wait_for_shutdown().await?;

    // stop fetching
    let _ = shutdown_tx.send(true);

    // close browser
    let _ = browser.close().await?;

    // join await
    let _ = handle.await?;

    let _ = trae_main.wait().await?;

    Ok(())
}

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
        on_finished: ActionChain::new().focus_task().custom(CustomActionExample),

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
