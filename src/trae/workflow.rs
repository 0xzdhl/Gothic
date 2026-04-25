use super::editor::TraeEditor;
use super::types::{ActionChain, ActionOp, TaskStatusChangeEvent, TaskWorkflow, TraeTask};
use crate::trae::{ActionContext, CustomAction};
use anyhow::Result;
use std::collections::HashMap;
use tokio::time::{Duration, sleep};
use tracing::info;

/// Compare the previous reconciled task list with the latest reconciled task list.
///
/// The key rule is that identity comes from `task_id`, not from `title` or `index`.
/// This lets us emit two independent events for duplicate titles as long as the
/// reconciliation step has already preserved distinct ids for them.
pub fn diff_task_status_changes(
    previous: &[TraeTask],
    latest: &[TraeTask],
) -> Vec<TaskStatusChangeEvent> {
    let previous_by_id = previous
        .iter()
        .map(|task| (task.task_id, task))
        .collect::<HashMap<_, _>>();

    latest
        .iter()
        .filter_map(|task| {
            if let Some(previous_task) = previous_by_id.get(&task.task_id) {
                return (previous_task.status != task.status).then(|| TaskStatusChangeEvent {
                    task: task.clone(),
                    previous_status: Some(previous_task.status),
                });
            }

            // A brand-new task has no previous status. We only emit an initial event
            // when the task is already actionable for automation, which keeps startup
            // noise low for tasks that are simply still running.
            (task.is_terminal() || task.is_waiting_for_hitl()).then(|| TaskStatusChangeEvent {
                task: task.clone(),
                previous_status: None,
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
            // Every task-scoped action goes through a `*_by_id` method so the editor
            // can resolve the freshest sidebar index immediately before touching the UI.
            ActionOp::FocusTask => editor.focus_task_by_id(task.task_id).await?,
            ActionOp::FocusChatInput => editor.focus_chat_input().await?,
            ActionOp::ClearChatInput => editor.clear_chat_input().await?,
            ActionOp::TypeText(text) => editor.insert_text_to_focused_input(text).await?,
            ActionOp::PressEnter => editor.click_send_button_by_id(task.task_id).await?,
            ActionOp::ClickSelector(selector) => editor.click_element_by_selector(selector).await?,
            ActionOp::ClickButtonByText(text) => editor.click_button_by_text(text).await?,
            ActionOp::WaitForSelector {
                selector,
                timeout_ms,
            } => editor.wait_until_selector(selector, *timeout_ms).await?,
            ActionOp::SleepMs(ms) => sleep(Duration::from_millis(*ms)).await,
            ActionOp::AllowCommand => editor.allow_command_by_id(task.task_id).await?,
            ActionOp::RejectCommand => editor.reject_command_by_id(task.task_id).await?,
            // Once a task enters WaitingForHITL, the workflow no longer guesses
            // which concrete prompt is visible. The editor reads the live DOM
            // and dispatches to the appropriate command or questionnaire handler.
            ActionOp::HandleHumanInLoop => editor.handle_human_in_loop_by_id(task.task_id).await?,
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
            ctx.editor.focus_task_by_id(ctx.task.task_id).await?;
            ctx.editor.focus_chat_input().await?;
            ctx.editor.clear_chat_input().await?;
            info!("Custom action triggered");
            Ok(())
        })
    }
}
