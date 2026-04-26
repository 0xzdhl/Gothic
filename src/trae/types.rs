use std::{fmt::Debug, pin::Pin, sync::Arc};

use crate::trae::TraeEditor;
use anyhow::Error;
use arboard::Clipboard;

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum TraeEditorMode {
    SOLO,
    IDE,
}

#[derive(Debug, Clone, Copy)]
pub enum TraeEditorPrebuiltSoloAgent {
    Coder,
    Builder,
}

pub enum TraeSoloTaskFeedback {
    Good,
    Bad,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TraeTaskStatus {
    Idle,
    Running,
    Interrupted,
    WaitingForHITL,
    Finished,
}

impl TraeTaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TraeTaskStatus::Idle => "Idle",
            TraeTaskStatus::Running => "Running",
            TraeTaskStatus::Interrupted => "Interrupted",
            TraeTaskStatus::WaitingForHITL => "WaitingForHITL",
            TraeTaskStatus::Finished => "Finished",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TraeTask {
    /// Stable in-memory identity used to track the same logical task across UI reorders.
    ///
    /// This value is assigned by our reconciliation layer, not by Trae itself.
    /// Unlike `index`, it is expected to remain stable for the lifetime of the process.
    pub task_id: u64,
    /// Human-visible title shown in the sidebar.
    pub title: String,
    pub status: TraeTaskStatus,
    /// Whether the sidebar item is currently selected in the UI.
    pub selected: bool,
    /// Current sidebar position in the latest UI snapshot.
    ///
    /// This is intentionally mutable because Trae can reorder tasks after creation,
    /// after terminal-task follow-up input, or while other tasks are inserted above it.
    pub index: usize,
}

impl TraeTask {
    pub fn is_running(&self) -> bool {
        matches!(self.status, TraeTaskStatus::Running)
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.status, TraeTaskStatus::Finished)
    }

    pub fn is_waiting_for_hitl(&self) -> bool {
        matches!(self.status, TraeTaskStatus::WaitingForHITL)
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            TraeTaskStatus::Interrupted | TraeTaskStatus::Finished
        )
    }
}

pub struct TraeTaskHandler<'a> {
    editor: &'a TraeEditor,
    snapshot: TraeTask,
}

impl<'a> TraeTaskHandler<'a> {
    pub fn new(editor: &'a TraeEditor, snapshot: TraeTask) -> Self {
        Self { editor, snapshot }
    }

    pub fn task(&self) -> &TraeTask {
        &self.snapshot
    }

    pub fn title(&self) -> &str {
        &self.snapshot.title
    }

    pub fn task_id(&self) -> u64 {
        self.snapshot.task_id
    }

    pub fn status(&self) -> TraeTaskStatus {
        self.snapshot.status
    }

    pub fn index(&self) -> usize {
        self.snapshot.index
    }

    pub fn is_selected(&self) -> bool {
        self.snapshot.selected
    }

    pub fn is_finished(&self) -> bool {
        self.snapshot.is_finished()
    }

    pub async fn refresh(&self) -> Result<TraeTaskHandler<'a>, Error> {
        self.editor
            .get_task_handle_by_id(self.snapshot.task_id)
            .await
    }

    pub async fn select(&self) -> Result<(), Error> {
        let _ui_guard = self.editor.acquire_ui_lock().await;
        self.editor.select_task_by_id(self.task_id()).await
    }

    pub async fn type_content(&self, content: &str) -> Result<(), Error> {
        let _ui_guard = self.editor.acquire_ui_lock().await;
        let _ = self.editor.select_task_by_id(self.task_id()).await?;
        self.editor.type_content_to_chat_input(content).await
    }

    pub async fn copy_summary(&self) -> Result<String, Error> {
        let _ui_guard = self.editor.acquire_ui_lock().await;
        // Always refocus the latest location of the task before reading task-scoped UI.
        let _ = self.editor.select_task_by_id(self.task_id()).await?;

        let _ = self.editor.copy_task_summary_by_id(self.task_id()).await?;

        // read summary from clipboard
        let mut clipboard = Clipboard::new()?;

        let clipboard_text = clipboard.get_text()?;

        Ok(clipboard_text)
    }

    pub async fn retry_task(&self) -> Result<(), Error> {
        let _ui_guard = self.editor.acquire_ui_lock().await;
        // Retry operates on the currently focused task panel, so refresh the focus first.
        let _ = self.editor.select_task_by_id(self.task_id()).await?;
        self.editor.retry_task_by_id(self.task_id()).await
    }

    pub async fn feedback(&self, feedback: TraeSoloTaskFeedback) -> Result<(), Error> {
        let _ui_guard = self.editor.acquire_ui_lock().await;
        // Feedback buttons belong to the active task panel, not to a stable DOM node.
        let _ = self.editor.select_task_by_id(self.task_id()).await?;
        self.editor
            .feedback_task_by_id(self.task_id(), feedback)
            .await
    }

    pub async fn terminate(&self) -> Result<(), Error> {
        let _ui_guard = self.editor.acquire_ui_lock().await;
        let _ = self.editor.select_task_by_id(self.task_id()).await?;

        self.editor.terminate_task_by_id(self.task_id()).await
    }

    pub async fn trigger_send(&self) -> Result<(), Error> {
        let _ui_guard = self.editor.acquire_ui_lock().await;
        let _ = self.editor.select_task_by_id(self.task_id()).await?;

        self.editor.click_send_button_by_id(self.task_id()).await
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskStatusChangeEvent {
    pub task: TraeTask,
    pub previous_status: Option<TraeTaskStatus>,
}

impl TaskStatusChangeEvent {
    pub fn current_status(&self) -> TraeTaskStatus {
        self.task.status
    }

    pub fn task_id(&self) -> u64 {
        self.task.task_id
    }
}

#[derive(Debug, Clone)]
pub enum ActionOp {
    FocusTask,
    FocusChatInput,
    ClearChatInput,
    /// The action only triggered `click` behavior, not creating task actually
    NewTask,
    TypeText(String),
    PressEnter,
    ClickSelector(String),
    ClickButtonByText(String),
    WaitForSelector {
        selector: String,
        timeout_ms: u64,
    },
    AllowCommand,
    RejectCommand,
    // Unified WaitingForHITL entry point.
    // Instead of asking the workflow to predict whether the current prompt is a
    // command card or a questionnaire, we defer that decision to `editor`,
    // where we can inspect the live DOM at execution time.
    HandleHumanInLoop,
    SleepMs(u64),
    Custom(Arc<dyn CustomAction>),
}

pub struct ActionContext<'a> {
    pub editor: &'a TraeEditor,
    pub task: &'a TraeTask,
    pub event: &'a TaskStatusChangeEvent,
}

pub type ActionFuture<'a> = Pin<Box<dyn Future<Output = Result<(), Error>> + Send + 'a>>;

pub trait CustomAction: Send + Sync + Debug {
    fn name(&self) -> &'static str;
    fn run<'a>(&'a self, ctx: ActionContext<'a>) -> ActionFuture<'a>;
}

#[derive(Debug, Clone)]
pub struct ActionChain {
    pub steps: Vec<ActionOp>,
}

impl ActionChain {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    pub fn then(mut self, step: ActionOp) -> Self {
        self.steps.push(step);
        self
    }

    pub fn new_task(self) -> Self {
        self.then(ActionOp::NewTask)
    }

    pub fn focus_task(self) -> Self {
        self.then(ActionOp::FocusTask)
    }

    pub fn focus_chat_input(self) -> Self {
        self.then(ActionOp::FocusChatInput)
    }
    pub fn clear_chat_input(self) -> Self {
        self.then(ActionOp::ClearChatInput)
    }
    pub fn type_text(self, text: impl Into<String>) -> Self {
        self.then(ActionOp::TypeText(text.into()))
    }
    pub fn press_enter(self) -> Self {
        self.then(ActionOp::PressEnter)
    }
    pub fn click_selector(self, selector: impl Into<String>) -> Self {
        self.then(ActionOp::ClickSelector(selector.into()))
    }
    pub fn click_button_by_text(self, text: impl Into<String>) -> Self {
        self.then(ActionOp::ClickButtonByText(text.into()))
    }
    pub fn wait_for_selector(self, selector: impl Into<String>, timeout_ms: u64) -> Self {
        self.then(ActionOp::WaitForSelector {
            selector: selector.into(),
            timeout_ms,
        })
    }
    pub fn sleep_ms(self, ms: u64) -> Self {
        self.then(ActionOp::SleepMs(ms))
    }

    pub fn allow_command(self) -> Self {
        self.then(ActionOp::AllowCommand)
    }

    pub fn reject_command(self) -> Self {
        self.then(ActionOp::RejectCommand)
    }

    // Provide a semantic builder for the unified HITL path so callers do not
    // need to manually compose Allow/Reject actions up front.
    pub fn handle_human_in_loop(self) -> Self {
        self.then(ActionOp::HandleHumanInLoop)
    }

    pub fn custom<A: CustomAction + 'static>(mut self, action: A) -> Self {
        self.steps.push(ActionOp::Custom(Arc::new(action)));
        self
    }
}

#[derive(Debug, Clone)]
pub struct TaskWorkflow {
    pub on_finished: ActionChain,
    pub on_interrupted: ActionChain,
    pub on_waiting_for_hitl: ActionChain,
}

impl TaskWorkflow {
    pub fn chain_for_status(&self, status: TraeTaskStatus) -> Option<&ActionChain> {
        match status {
            TraeTaskStatus::Finished => Some(&self.on_finished),
            TraeTaskStatus::Interrupted => Some(&self.on_interrupted),
            TraeTaskStatus::WaitingForHITL => Some(&self.on_waiting_for_hitl),
            TraeTaskStatus::Idle | TraeTaskStatus::Running => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum InitialTaskPolicy {
    Ignore,
    EmitAll,
    EmitTerminalAndWaiting,
}

impl InitialTaskPolicy {
    pub fn should_emit(&self, status: TraeTaskStatus) -> bool {
        match self {
            InitialTaskPolicy::Ignore => false,
            InitialTaskPolicy::EmitAll => true,
            InitialTaskPolicy::EmitTerminalAndWaiting => matches!(
                status,
                // TraeTaskStatus::Finished
                |TraeTaskStatus::Interrupted| TraeTaskStatus::WaitingForHITL
            ),
        }
    }
}
