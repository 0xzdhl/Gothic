use anyhow::Result;
use chromiumoxide::Browser;
use futures::StreamExt;
use gothic::config::Config;
use gothic::trae::{
    ActionChain, CustomActionExample, InitialTaskPolicy, TaskWorkflow, TraeEditor, TraeEditorMode,
};
use gothic::utils::{wait_for_debug_port, wait_for_shutdown};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::watch;
use tokio::time::{Duration, sleep};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load()?;

    let mut trae_main = Command::new(config.trae_executable_path)
        .arg("--remote-debugging-port=9222")
        .arg("--no-sandbox")
        .stdout(Stdio::null()) // inherit current stream
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;

    // let trae_pid = trae_main.id().expect("Cannot get Trae PID.");

    println!("Hello, world!");

    wait_for_debug_port(9222, Duration::from_secs(30)).await?;

    // connect to CDP
    let (mut browser, mut handler) = Browser::connect("http://127.0.0.1:9222").await?;
    println!("Successfully connect to Trae via CDP: 127.0.0.1:9222");

    // spawn a new task that continuously polls the handler
    let handle = tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            match event {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("Handler error: {e}");
                    break;
                }
            }
        }
    });

    let trae_editor_builder = TraeEditor::new();

    let mut trae_editor = trae_editor_builder.build(&mut browser).await;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    println!("Current Trae Mode: {:?}", trae_editor.mode);
    // switch mode
    trae_editor.switch_editor_mode(TraeEditorMode::SOLO).await?;

    // create a new task
    {
        quick_task("翻译：我喜欢使用Python编程为日语。", &trae_editor).await;
        // quick_task("帮我做一个淘宝网，我需要全部的功能", &trae_editor).await;
        // quick_task(
        //     "写一个小红书脚本，抓取特定关键词的热门帖子数据",
        //     &trae_editor,
        // )
        // .await;
        // quick_task("我想要做一个二手交易网站，我该怎么设计？", &trae_editor).await;
        // quick_task(
        //     "我是一个编程小白, 我想要学习Typescript, 我该从哪里开始?",
        //     &trae_editor,
        // )
        // .await;
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

    println!("Tasks: {:#?}", tasks);

    // click the second task

    if tasks.len() > 3 {
        let second_task = tasks.get(1).unwrap();
        let second_task_handler = arc_editor
            .get_task_handle_by_index(second_task.index)
            .await?;

        // trigger selection
        second_task_handler.select().await?;

        // try type something in it
        second_task_handler.type_content("fuck everything.").await?;

        // switch to third item, copy summary text

        // let third_task = tasks.get(2).unwrap();
        // let third_task_handler = arc_editor
        //     .get_task_handle_by_index(third_task.index)
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
        Ok(_) => println!("✅️ Task executed successfully. ({})", prompt),
        Err(e) => eprintln!("Task execution failed: {e}"),
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
            .press_enter(),
        // .press_enter(),
        on_waiting_for_hitl: ActionChain::new()
            .focus_task()
            .wait_for_selector(r#"button[data-testid="hitl-primary-button"]"#, 30_000)
            .click_selector(r#"button[data-testid="hitl-primary-button"]"#),
    }
}
