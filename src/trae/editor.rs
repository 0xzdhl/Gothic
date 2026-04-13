use crate::consts::*;
use crate::trae::task::{TraeSoloTask, TraeSoloTaskInner};
use crate::trae::types::*;
use anyhow::{Error, Result};
use chromiumoxide::{Browser, Page, cdp::browser_protocol::target::TargetInfo};
use std::marker::PhantomData;
use tokio::time::{Duration, sleep};

#[derive(Debug)]
pub struct TraeEditor {
    pub(crate) main_page: Page,
    pub(crate) target: TargetInfo,
    pub(crate) prebuilt_agent: TraeEditorPrebuiltSoloAgent,
}

pub struct TraeEditorBuilder;

impl TraeEditorBuilder {
    pub async fn build(&self, browser: &mut Browser) -> TraeEditor {
        let targets = browser.fetch_targets().await.expect("Fetch targets error.");

        sleep(Duration::from_millis(2000)).await;

        let mut filtered_target: Vec<TargetInfo> = targets
            .into_iter()
            .filter(|t| t.url == TRAE_MAIN_PAGE_URL)
            .collect();

        let main_target = filtered_target
            .pop()
            .expect("Cannot get the main target of Trae.");

        let pages = browser
            .pages()
            .await
            .expect("Cannot get pages from browser instance.");

        let main_page = browser
            .get_page(main_target.target_id.clone())
            .await
            .expect(&format!(
                "Cannot get the main page of Trae. filtered targets: {:#?}, main_target: {:#?}, pages: {:#?}",
                filtered_target, main_target, pages
            ));

        return TraeEditor {
            target: main_target,
            main_page: main_page,
            prebuilt_agent: TraeEditorPrebuiltSoloAgent::Coder,
        };
    }
}

impl TraeEditor {
    pub fn new() -> TraeEditorBuilder {
        TraeEditorBuilder {}
    }

    pub async fn get_main_page(&self) -> &Page {
        return &self.main_page;
    }

    pub async fn get_current_editor_mode(&self) -> Result<TraeEditorMode, Error> {
        let trae_mode_badge_element = self.main_page.find_element("div.fixed-titlebar-container div.icube-mode-tab > div.icube-tooltip-container > div.icube-tooltip-text.icube-simple-style").await.expect("Cannot locate Trae editor mode badge.");

        let mode_description = trae_mode_badge_element
            .inner_html()
            .await
            .expect("Cannot get the Trae mode badge text node")
            .expect("Cannot get Trae mode text description.");

        if mode_description.eq(TRAE_SOLO_MODE_TEXT_LABEL) {
            Ok(TraeEditorMode::IDE)
        } else if mode_description.eq(TRAE_IDE_MODE_TEXT_LABEL) {
            Ok(TraeEditorMode::SOLO)
        } else {
            Err(Error::msg("Cannot get the current editor mode"))
        }
    }

    pub async fn switch_editor_mode(&self, mode: TraeEditorMode) -> Result<(), Error> {
        let current_mode = self.get_current_editor_mode().await?;

        if current_mode == mode {
            return Ok(());
        }

        let trae_mode_tab_switch = self.main_page.find_element("div.fixed-titlebar-container div.icube-mode-tab > div.icube-mode-tab-container > div.icube-mode-tab-switch").await.expect("Cannot locate Trae editor mode switch tab.");
        trae_mode_tab_switch.click().await?;

        Ok(())
    }

    pub async fn create_new_task<'a>(&'a self, prompt: String) -> TraeSoloTask<'a> {
        TraeSoloTask::Idle(TraeSoloTaskInner::<Idle>::new(prompt, self))
    }

    pub fn set_default_prebuilt_solo_agent(&mut self, agent: TraeEditorPrebuiltSoloAgent) {
        self.prebuilt_agent = agent;
    }

    pub fn get_default_prebuilt_solo_agent(&self) -> TraeEditorPrebuiltSoloAgent {
        self.prebuilt_agent
    }

    // private methods

    // get tasks from sidebar
    pub async fn get_tasks(&'_ self) -> Result<Vec<TraeSoloTask<'_>>, Error> {
        let current_mode = self.get_current_editor_mode().await?;

        if current_mode != TraeEditorMode::SOLO {
            return Err(Error::msg(
                "Cannot get tasks under IDE mode, please switch to SOLO mode.",
            ));
        }

        let task_container = self
            .main_page
            .find_element("#solo-ai-sidebar-content div[class*=task-items-list]")
            .await
            .expect("Cannot get task container.");

        let task_items = task_container
            .find_elements("div[class*=task-item]")
            .await
            .expect("Cannot get task items from container.");

        let mut tasks: Vec<TraeSoloTask> = Vec::new();
        // TODO
        // 1. WaitingForHITL
        // 2. Finished
        for t in task_items.iter() {
            let raw_task_state = t
                .find_element("div[class*=task-type-wrap")
                .await
                .expect(&format!("Cannot get task type: {:#?}", t))
                .inner_html()
                .await
                .unwrap_or_default()
                .unwrap_or_else(|| {
                    println!("Trying to get task type label failed, the value is None");
                    return "".to_string();
                });

            let task_title = t
                .find_element("span[class*=task-title]")
                .await
                .expect(&format!("Cannot get task title: {:#?}", t))
                .inner_html()
                .await
                .unwrap_or_default()
                .unwrap_or_else(|| {
                    println!("Trying to get task title label failed, the value is None");
                    return "".to_string();
                });

            let task = match raw_task_state.as_str() {
                TRAE_SOLO_TASK_INTERRUPTED_LABEL => TraeSoloTask::Interrupted(TraeSoloTaskInner {
                    _state: PhantomData,
                    editor: self,
                    prompt: None,
                    title: task_title,
                }),
                TRAE_SOLO_TASK_RUNNING_LABEL => TraeSoloTask::Running(TraeSoloTaskInner {
                    _state: PhantomData,
                    editor: self,
                    prompt: None,
                    title: task_title,
                }),
                _ => TraeSoloTask::Idle(TraeSoloTaskInner {
                    _state: PhantomData,
                    editor: self,
                    prompt: None,
                    title: task_title,
                }),
            };

            tasks.push(task);
        }

        Ok(tasks)
    }
}
