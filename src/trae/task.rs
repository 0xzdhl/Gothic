use crate::trae::TaskListHint;
use crate::trae::editor::TraeEditor;
use crate::trae::types::*;
use anyhow::{Error, Result};
use tokio::time::{Duration, sleep};
use tracing::instrument;

#[derive(Debug)]
pub struct NewTraeTask<'a> {
    editor: &'a TraeEditor,
    prompt: String,
}

impl<'a> NewTraeTask<'a> {
    pub fn new(editor: &'a TraeEditor, prompt: String) -> Self {
        Self { editor, prompt }
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub async fn optimize_prompt(&self) -> Result<(), Error> {
        Err(Error::msg(
            "`optimize_prompt` is not implemented in the simplified task API yet.",
        ))
    }

    #[instrument(skip(self), fields(prompt = %self.prompt()))]
    pub async fn execute(&self) -> Result<(), Error> {
        let _ui_guard = self.editor.acquire_ui_lock().await;

        self.ensure_solo_mode().await?;

        self.editor.click_create_task_button().await?;
        self.editor
            .type_content_to_chat_input(self.prompt.as_str())
            .await?;

        sleep(Duration::from_millis(500)).await;

        self.editor
            .click_element_by_selector("button[class*=chat-input-v2-send-button]")
            .await?;
        self.editor
            .set_task_list_hint(TaskListHint::NewTaskAtFront)
            .await;

        sleep(Duration::from_millis(1000)).await;

        let _ = self.editor.refresh_tasks().await;

        Ok(())
    }

    async fn ensure_solo_mode(&self) -> Result<(), Error> {
        let mode = self.editor.get_current_mode();

        if *mode != TraeEditorMode::SOLO {
            return Err(Error::msg(
                "Cannot create task under IDE mode, please switch to SOLO mode first.",
            ));
        }

        Ok(())
    }
}
