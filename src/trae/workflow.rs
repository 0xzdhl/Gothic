use super::editor::TraeEditor;
use super::types::{ActionChain, ActionOp, TaskStatusChangeEvent, TaskWorkflow, TraeTask};
use crate::trae::{ActionContext, CustomAction};
use anyhow::Result;
use tokio::time::{Duration, sleep};

pub fn diff_task_status_changes(
    previous: &[TraeTask],
    latest: &[TraeTask],
) -> Vec<TaskStatusChangeEvent> {
    latest
        .iter()
        .filter_map(|task| {
            let previous_task = previous.get(task.index)?;

            if previous_task.title != task.title {
                return None;
            }

            (previous_task.status != task.status).then(|| TaskStatusChangeEvent {
                task: task.clone(),
                previous_status: Some(previous_task.status),
            })
        })
        .collect()
}

pub async fn execute_action_chain(
    editor: &TraeEditor,
    event: &TaskStatusChangeEvent,
    chain: &ActionChain,
) -> Result<()> {
    let task = &event.task;

    for step in &chain.steps {
        match step {
            ActionOp::FocusTask => editor.focus_task_by_index(task.index).await?,
            ActionOp::FocusChatInput => editor.focus_chat_input().await?,
            ActionOp::ClearChatInput => editor.clear_chat_input().await?,
            ActionOp::TypeText(text) => editor.insert_text_to_focused_input(text).await?,
            ActionOp::PressEnter => editor.click_send_button_by_index(task.index).await?,
            ActionOp::ClickSelector(selector) => editor.click_element_by_selector(selector).await?,
            ActionOp::ClickButtonByText(text) => editor.click_button_by_text(text).await?,
            ActionOp::WaitForSelector {
                selector,
                timeout_ms,
            } => editor.wait_until_selector(selector, *timeout_ms).await?,
            ActionOp::SleepMs(ms) => sleep(Duration::from_millis(*ms)).await,
            ActionOp::AllowCommand => editor.allow_command_by_index(task.index).await?,
            ActionOp::RejectCommand => editor.reject_command_by_index(task.index).await?,
            // WaitingForHITL 进入这里后，不再由 workflow 假设具体卡片类型，
            // 而是交给 editor 读取 DOM 后决定走 command 还是 questionnaire 分支。
            ActionOp::HandleHumanInLoop => editor.handle_human_in_loop_by_index(task.index).await?,
            ActionOp::Custom(action) => {
                action
                    .run(ActionContext {
                        editor,
                        task,
                        event,
                    })
                    .await?;
            }
        }
    }

    Ok(())
}

pub async fn handle_task_status_change(
    editor: &TraeEditor,
    workflow: &TaskWorkflow,
    event: &TaskStatusChangeEvent,
) -> Result<()> {
    let Some(chain) = workflow.chain_for_status(event.current_status()) else {
        return Ok(());
    };

    if chain.steps.is_empty() {
        return Ok(());
    }

    execute_action_chain(editor, event, chain).await
}

/// Custom Action Example
/// 1. Focus task
/// 2. Type "Hello, world"
#[derive(Debug)]
pub struct CustomActionExample;

impl CustomAction for CustomActionExample {
    fn name(&self) -> &'static str {
        "custom_action_example"
    }

    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> super::ActionFuture<'a> {
        Box::pin(async move {
            ctx.editor.focus_task_by_index(ctx.task.index).await?;
            ctx.editor.focus_chat_input().await?;
            ctx.editor.clear_chat_input().await?;
            println!("Custom Action Triggered");
            Ok(())
        })
    }
}
