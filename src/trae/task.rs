use crate::trae::types::*;
use crate::utils::wait_for_selector;
use crate::{consts::DEFAULT_SELECTOR_TIMEOUT, trae::editor::TraeEditor};
use anyhow::{Error, Result};
use chromiumoxide::cdp::browser_protocol::input::InsertTextParams;
use tokio::time::{Duration, sleep};

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

    pub async fn execute(&self) -> Result<(), Error> {
        self.ensure_solo_mode().await?;

        let _ = wait_for_selector(
            &self.editor.main_page,
            "div.chat-content-container",
            Duration::from_millis(1000 * 60),
        )
        .await?;

        let create_task_button = self
            .editor
            .main_page
            .find_element(r#"#solo-ai-sidebar-content div[class*="new-task-button"]"#)
            .await?;

        // click create button
        create_task_button.click().await?;

        self.wait_until_task_creation_page_ready().await?;

        let chat_input_element = wait_for_selector(
            &self.editor.main_page,
            "#agent-chat-view div.chat-input-wrapper div.chat-input-v2-input-box-editable",
            Duration::from_millis(1000 * 60),
        )
        .await?;

        chat_input_element.click().await?;

        // clear input first
        self.editor.clear_chat_input().await?;

        self.editor
            .main_page
            .execute(InsertTextParams::new(self.prompt.as_str()))
            .await?;

        sleep(Duration::from_millis(500)).await;

        chat_input_element.press_key("Enter").await?;

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

    async fn wait_until_task_creation_page_ready(&self) -> Result<(), Error> {
        let _ = wait_for_selector(
            &self.editor.main_page,
            "div.welcome-page-solo-agent-title",
            Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
        )
        .await?;

        Ok(())
    }
}
