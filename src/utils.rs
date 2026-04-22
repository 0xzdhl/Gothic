use std::path::Path;

use anyhow::{Error, Result, bail};
use chromiumoxide::{Element, Page};
use tokio::{
    signal,
    time::{Duration, Instant, sleep},
};

use crate::trae::{ActionChain, CustomActionExample, TaskWorkflow, TraeEditor};

pub async fn wait_for_debug_port(
    port: u16,
    timeout: Duration,
) -> Result<(), Box<dyn std::error::Error>> {
    let start = Instant::now(); // current time;
    while start.elapsed() < timeout {
        // try connect the port via TCP
        if tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .is_ok()
        {
            // sleep 1 sec after the port is connectable
            sleep(Duration::from_secs(1)).await;
            return Ok(());
        }
        sleep(Duration::from_millis(300)).await;
    }
    Err(format!("Timeout listening the CDP port: {}", port).into())
}

pub async fn wait_for_shutdown() -> Result<()> {
    signal::ctrl_c().await?;
    Ok(())
}

pub async fn wait_for_selector(
    page: &Page,
    selector: &str,
    timeout: Duration,
) -> Result<Element, Error> {
    let start = Instant::now();

    loop {
        let ele = page.find_element(selector).await;
        if ele.is_ok() {
            return Ok(ele?);
        }

        if start.elapsed() >= timeout {
            bail!("Timeout locating: {}", selector);
        }

        sleep(Duration::from_millis(300)).await;
    }
}

pub fn normalize_executable_path_for_cdp(raw_path: &str) -> Option<String> {
    let parent = Path::new(raw_path).parent()?;

    let mut dir = parent.to_string_lossy().into_owned();

    if dir.len() >= 2 && dir.as_bytes()[1] == b':' {
        dir[0..1].make_ascii_lowercase();
    }

    Some(dir.replace('\\', "/"))
}

pub fn build_task_workflow() -> TaskWorkflow {
    TaskWorkflow {
        on_finished: ActionChain::new().focus_task().custom(CustomActionExample),

        on_interrupted: ActionChain::new()
            .focus_task()
            .focus_chat_input()
            .clear_chat_input()
            .type_text("任务中断了，请说明阻塞点和下一步建议。")
            .press_enter(),

        on_waiting_for_hitl: ActionChain::new()
            .focus_task()
            .wait_for_selector(r#"button[data-testid="hitl-primary-button"]"#, 30_000)
            .click_selector(r#"button[data-testid="hitl-primary-button"]"#),
    }
}

pub async fn quick_task(prompt: &str, editor: &TraeEditor) {
    let task = editor.create_new_task(prompt.to_string()).await;

    // execute task
    match task.execute().await {
        Ok(_) => println!("✅️ Task executed successfully. ({})", prompt),
        Err(e) => eprintln!("Task execution failed: {e}"),
    }

    // sleep 1 sec
    sleep(Duration::from_millis(3000)).await;
}
