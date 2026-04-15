use crate::trae::TraeEditor;
use anyhow::Error;

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

#[derive(Debug, Clone, Copy)]
pub enum TraeTaskStatus {
    Idle,
    Running,
    Interrupted,
    WaitingForHITL,
    Finished,
}

#[derive(Debug, Clone)]
pub struct TraeTask {
    pub title: String,
    pub status: TraeTaskStatus,
    pub selected: bool,
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
            .get_task_handle_by_index(self.snapshot.index)
            .await
    }

    pub async fn select(&self) -> Result<(), Error> {
        self.editor.select_task_by_index(self.index()).await
    }

    pub async fn type_content(&self, content: &str) -> Result<(), Error> {
        self.editor.type_content_to_chat_input(content).await
    }
}
