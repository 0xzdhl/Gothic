use crate::config::{CommandStrategy, Config, QuestionStrategy};
use crate::trae::{
    NewTraeTask, TraeTask, TraeTaskStatus, diff_task_status_changes, handle_task_status_change,
    types::*,
};

use crate::trae::consts::*;
use crate::utils::{normalize_executable_path_for_cdp, wait_for_selector};
use anyhow::{Error, Result, bail};
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs, ResponseFormat,
    },
};
use chromiumoxide::Element;
use chromiumoxide::cdp::browser_protocol::input::InsertTextParams;
use chromiumoxide::{Browser, Page, cdp::browser_protocol::target::TargetInfo};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tokio::sync::watch::Receiver;
use tokio::time::{self, Duration, Instant, sleep};

// TODO: Refactor

#[derive(Debug)]
pub struct TraeEditor {
    pub(crate) main_page: Page,
    pub(crate) target: TargetInfo,
    pub(crate) prebuilt_agent: TraeEditorPrebuiltSoloAgent,
    pub mode: TraeEditorMode,
    pub(crate) tasks: RwLock<Vec<TraeTask>>,
    pub config: Config,
}

#[derive(Debug, Deserialize)]
struct TaskSnapshotFromUi {
    title: String,
    raw_status: String,
    selected: bool,
}

#[derive(Clone, Copy, Debug)]
enum CommandCardKind {
    Delete,
    Run,
}

impl CommandCardKind {
    fn from_class_name(class_name: &str) -> Self {
        if class_name.contains(DELETE_COMMAND_CARD_CLASS) {
            Self::Delete
        } else {
            Self::Run
        }
    }

    fn raw_command_selector(self) -> &'static str {
        match self {
            Self::Delete => DELETE_COMMAND_TEXT_SELECTOR,
            Self::Run => RUN_COMMAND_TEXT_SELECTOR,
        }
    }

    fn action_button_selector(self, decision: CommandDecision) -> &'static str {
        match (self, decision) {
            (Self::Delete, CommandDecision::Allow) => DELETE_COMMAND_ALLOW_SELECTOR,
            (Self::Delete, CommandDecision::Reject) => DELETE_COMMAND_REJECT_SELECTOR,
            (Self::Run, CommandDecision::Allow) => RUN_COMMAND_ALLOW_SELECTOR,
            (Self::Run, CommandDecision::Reject) => RUN_COMMAND_REJECT_SELECTOR,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum CommandDecision {
    Allow,
    Reject,
}

impl CommandDecision {
    fn log_label(self) -> &'static str {
        match self {
            Self::Allow => "Allowed",
            Self::Reject => "Rejected",
        }
    }
}

#[derive(Debug)]
struct PendingCommandAction {
    kind: CommandCardKind,
    raw_command: String,
}

/// Human in the loop kind
/// Command: shell execution, delete files, .etc
/// Questionnaire: model will ask human a few questions
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HitlPromptKind {
    Command,
    Questionnaire,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct QuestionnaireOption {
    title: String,
    description: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Questionnaire {
    context: String,
    question: String,
    options: Vec<QuestionnaireOption>,
    is_multiple: bool,
    // if the option contains a textarea
    has_text_input: bool,
    text_input_placeholder: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LlmQuestionSelection {
    option_indices: Vec<usize>,
}

pub async fn get_current_editor_mode(page: &Page) -> Result<TraeEditorMode, Error> {
    let trae_mode_badge_element = wait_for_selector(page, "div.fixed-titlebar-container div.icube-mode-tab > div.icube-tooltip-container > div.icube-tooltip-text.icube-simple-style", Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT)).await.expect("Cannot locate Trae editor mode badge.");

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
        Err(Error::msg(format!(
            "Cannot get the current editor mode, description: {}",
            mode_description
        )))
    }
}

fn parse_task_status(raw_status: &str) -> TraeTaskStatus {
    match raw_status {
        TRAE_SOLO_TASK_RUNNING_LABEL => TraeTaskStatus::Running,
        TRAE_SOLO_TASK_INTERRUPTED_LABEL => TraeTaskStatus::Interrupted,
        TRAE_SOLO_TASK_FINISHED_LABEL => TraeTaskStatus::Finished,
        TRAE_SOLO_TASK_WAITING_FOR_HITL_LABEL => TraeTaskStatus::WaitingForHITL,
        _ => TraeTaskStatus::Idle,
    }
}

pub struct TraeEditorBuilder;

impl TraeEditorBuilder {
    pub async fn build(&self, browser: &mut Browser, config: Config) -> TraeEditor {
        let targets = browser.fetch_targets().await.expect("Fetch targets error.");

        sleep(Duration::from_millis(2000)).await;

        let normalized_path =
            normalize_executable_path_for_cdp(&config.trae_executable_path).unwrap();

        let mut filtered_target: Vec<TargetInfo> = targets
            .into_iter()
            .filter(|t| t.url.contains(&format!("vscode-file://vscode-app/{}/resources/app/out/vs/code/electron-browser/workbench/workbench.html",normalized_path)))
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

        // get the current mode
        let current_mode = get_current_editor_mode(&main_page)
            .await
            .expect("Cannot get current mode when initializing.");

        return TraeEditor {
            target: main_target,
            main_page: main_page,
            prebuilt_agent: TraeEditorPrebuiltSoloAgent::Coder,
            mode: current_mode,
            config,
            tasks: RwLock::new(Vec::new()),
        };
    }
}

impl TraeEditor {
    pub fn new() -> TraeEditorBuilder {
        TraeEditorBuilder {}
    }

    pub fn get_current_mode(&self) -> &TraeEditorMode {
        &self.mode
    }

    pub async fn get_main_page(&self) -> &Page {
        return &self.main_page;
    }

    pub async fn switch_editor_mode(&mut self, mode: TraeEditorMode) -> Result<(), Error> {
        if self.mode == mode {
            return Ok(());
        }

        let trae_mode_tab_switch = self.main_page.find_element("div.fixed-titlebar-container div.icube-mode-tab > div.icube-mode-tab-container > div.icube-mode-tab-switch").await.expect("Cannot locate Trae editor mode switch tab.");
        trae_mode_tab_switch.click().await?;

        // update current mode

        match self.mode {
            TraeEditorMode::IDE => {
                self.mode = TraeEditorMode::SOLO;
            }
            TraeEditorMode::SOLO => {
                self.mode = TraeEditorMode::IDE;
            }
        }

        Ok(())
    }

    pub async fn create_new_task(&self, prompt: impl Into<String>) -> NewTraeTask<'_> {
        NewTraeTask::new(self, prompt.into())
    }

    pub fn set_default_prebuilt_solo_agent(&mut self, agent: TraeEditorPrebuiltSoloAgent) {
        self.prebuilt_agent = agent;
    }

    pub fn get_default_prebuilt_solo_agent(&self) -> TraeEditorPrebuiltSoloAgent {
        self.prebuilt_agent
    }

    // private methods
    // get tasks from sidebar
    pub async fn fetch_tasks_from_ui(&self) -> Result<Vec<TraeTask>, Error> {
        if self.mode != TraeEditorMode::SOLO {
            return Err(Error::msg(
                "Cannot get tasks under IDE mode, please switch to SOLO mode.",
            ));
        }

        let _ = wait_for_selector(
            &self.main_page,
            r#"div[class*=index-module__task-item___]"#,
            Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
        )
        .await
        .expect("Cannot get task items from page.");

        let task_snapshots: Vec<TaskSnapshotFromUi> = self
            .main_page
            .evaluate(
                r#"
        Array.from(document.querySelectorAll('div[class*="index-module__task-item___"]'))
          .map((item) => {
            const titleElement = item.querySelector('span[class*="task-title"]');
            const statusElement = item.querySelector('div[class*="task-type-wrap__status"]');

            return {
              title: (titleElement?.textContent ?? '').trim(),
              raw_status: (statusElement?.innerText ?? '').trim(),
              selected: item.className.includes('selected'),
            };
          })
        "#,
            )
            .await?
            .into_value()?;

        let mut tasks = Vec::with_capacity(task_snapshots.len());

        for (index, snapshot) in task_snapshots.into_iter().enumerate() {
            tasks.push(TraeTask {
                title: snapshot.title.trim().to_string(),
                status: parse_task_status(snapshot.raw_status.as_str()),
                selected: snapshot.selected,
                index,
            });
        }

        Ok(tasks)
    }

    pub async fn refresh_tasks(&self) -> Result<Vec<TraeTask>, Error> {
        let latest = self.fetch_tasks_from_ui().await?;

        let mut guard = self.tasks.write().await;
        *guard = latest.clone();

        Ok(latest)
    }

    pub async fn refresh_tasks_with_events(&self) -> Result<Vec<TaskStatusChangeEvent>, Error> {
        let previous = self.cached_tasks().await;
        let latest = self.fetch_tasks_from_ui().await?;
        let events = diff_task_status_changes(&previous, &latest);

        let mut guard = self.tasks.write().await;
        *guard = latest;

        Ok(events)
    }

    async fn bootstrap_tasks_with_events(
        &self,
        policy: InitialTaskPolicy,
    ) -> Result<Vec<TaskStatusChangeEvent>, Error> {
        let latest = self.fetch_tasks_from_ui().await?;

        let events = latest
            .iter()
            .filter(|task| policy.should_emit(task.status))
            .cloned()
            .map(|task| TaskStatusChangeEvent {
                task,
                previous_status: None,
            })
            .collect::<Vec<_>>();

        let mut guard = self.tasks.write().await;
        *guard = latest;
        Ok(events)
    }

    async fn dispatch_task_events(
        &self,
        workflow: &TaskWorkflow,
        events: Vec<TaskStatusChangeEvent>,
    ) {
        for event in events {
            println!(
                "Task event: [{}] {} -> {}",
                event.task.title,
                event
                    .previous_status
                    .map(|s| s.as_str())
                    .unwrap_or("Initial"),
                event.current_status().as_str(),
            );

            if let Err(err) = handle_task_status_change(self, workflow, &event).await {
                eprintln!("handle_task_status_change failed: {err:?}")
            }
        }
    }

    pub async fn focus_task_by_index(&self, index: usize) -> Result<(), Error> {
        self.select_task_by_index(index).await?;
        sleep(Duration::from_millis(300)).await;
        Ok(())
    }

    pub async fn focus_chat_input(&self) -> Result<(), Error> {
        let chat_input_element = wait_for_selector(
            &self.main_page,
            CHAT_INPUT_SELECTOR,
            Duration::from_millis(1000 * 60),
        )
        .await?;

        chat_input_element.click().await?;
        sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    async fn select_all_chat_input_content(&self) -> Result<bool, Error> {
        Ok(self
            .main_page
            .evaluate(format!(
                r#"
        (() => {{
            const editor = document.querySelector({selector:?});
            if (!(editor instanceof HTMLElement)) {{
                return false;
            }}

            editor.focus();

            const selection = window.getSelection();
            if (!selection) {{
                return false;
            }}

            const range = document.createRange();
            range.selectNodeContents(editor);
            selection.removeAllRanges();
            selection.addRange(range);

            return true;
        }})()
        "#,
                selector = CHAT_INPUT_SELECTOR
            ))
            .await?
            .into_value()?)
    }

    pub async fn clear_chat_input(&self) -> Result<(), Error> {
        self.focus_chat_input().await?;
        let selected = self.select_all_chat_input_content().await?;

        if !selected {
            return Err(Error::msg("Cannot select the Trae chat input content."));
        }

        let chat_input_element = wait_for_selector(
            &self.main_page,
            CHAT_INPUT_SELECTOR,
            Duration::from_millis(1000 * 60),
        )
        .await?;

        sleep(Duration::from_millis(100)).await;
        chat_input_element.press_key("Backspace").await?;
        sleep(Duration::from_millis(200)).await;

        Ok(())
    }

    pub async fn insert_text_to_focused_input(&self, content: &str) -> Result<(), Error> {
        self.main_page
            .execute(InsertTextParams::new(content))
            .await?;
        sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    pub async fn wait_until_selector(&self, selector: &str, timeout_ms: u64) -> Result<(), Error> {
        let _ =
            wait_for_selector(&self.main_page, selector, Duration::from_millis(timeout_ms)).await?;
        Ok(())
    }

    pub async fn click_element_by_selector(&self, selector: &str) -> Result<(), Error> {
        let element = wait_for_selector(
            &self.main_page,
            selector,
            Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
        )
        .await?;

        element.click().await?;
        sleep(Duration::from_millis(300)).await;
        Ok(())
    }

    pub async fn click_button_by_text(&self, button_text: &str) -> Result<(), Error> {
        let button_text_literal = format!("{button_text:?}");

        let clicked: bool = self
            .main_page
            .evaluate(format!(
                r#"
        (() => {{
            const expectedText = {button_text_literal}.trim();
            const candidates = Array.from(document.querySelectorAll('button, [role="button"]'));
            const target = candidates.find(
                (node) => (node.textContent ?? node.innerText ?? '').trim() === expectedText
            );

            if (!target) {{
                return false;
            }}

            target.click();
            return true;
        }})()
        "#
            ))
            .await?
            .into_value()?;

        if !clicked {
            return Err(Error::msg(format!(
                "Cannot find button by text: {}",
                button_text
            )));
        }

        sleep(Duration::from_millis(300)).await;
        Ok(())
    }

    async fn try_click_button_by_text_in_scope(
        &self,
        scope_selector: &str,
        button_text: &str,
    ) -> Result<bool, Error> {
        let scope_selector_literal = format!("{scope_selector:?}");
        let button_text_literal = format!("{button_text:?}");

        Ok(self
            .main_page
            .evaluate(format!(
                r#"
        (() => {{
            const expectedText = {button_text_literal}.trim();

            for (const scope of document.querySelectorAll({scope_selector_literal})) {{
                const target = Array.from(scope.querySelectorAll('button, [role="button"]')).find(
                    (node) => (node.textContent ?? node.innerText ?? '').trim() === expectedText
                );

                if (target) {{
                    target.click();
                    return true;
                }}
            }}

            return false;
        }})()
        "#
            ))
            .await?
            .into_value()?)
    }

    async fn confirm_delete_command(&self) -> Result<(), Error> {
        let timeout = Duration::from_millis(COMMAND_ACTION_TIMEOUT_MS);
        let start = Instant::now();

        while start.elapsed() < timeout {
            if self
                .try_click_button_by_text_in_scope(
                    DELETE_CONFIRM_POPOVER_SELECTOR,
                    DELETE_CONFIRM_BUTTON_TEXT,
                )
                .await?
            {
                return Ok(());
            }

            sleep(Duration::from_millis(COMMAND_ACTION_POLL_INTERVAL_MS)).await;
        }

        Err(Error::msg(
            "Cannot find the delete confirmation button from popover.",
        ))
    }

    /// Get latest Trae tasks from UI.
    pub async fn get_tasks(&self) -> Result<Vec<TraeTask>, Error> {
        let latest = self.fetch_tasks_from_ui().await?;
        let mut guard = self.tasks.write().await;
        *guard = latest.clone();

        Ok(latest)
    }

    async fn get_command_card_kind(
        &self,
        command_container: &Element,
    ) -> Result<CommandCardKind, Error> {
        let element_class = command_container
            .attribute("class")
            .await?
            .unwrap_or_default();

        Ok(CommandCardKind::from_class_name(&element_class))
    }

    async fn get_raw_command_str(
        &self,
        command_kind: CommandCardKind,
        command_container: &Element,
        index: usize,
    ) -> Result<String, Error> {
        let raw_command = match wait_for_selector(
            &self.main_page,
            command_kind.raw_command_selector(),
            Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
        )
        .await
        {
            Ok(element) => element.inner_text().await?,
            Err(_) => command_container.inner_text().await?,
        };

        Ok(raw_command.unwrap_or_else(|| format!("Cannot get command str at index: {}", index)))
    }

    async fn resolve_pending_command_action(
        &self,
        index: usize,
    ) -> Result<PendingCommandAction, Error> {
        self.select_task_by_index(index).await?;
        let command_container = wait_for_selector(
            &self.main_page,
            COMMAND_CARD_SELECTOR,
            Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
        )
        .await?;
        let kind = self.get_command_card_kind(&command_container).await?;
        let raw_command = self
            .get_raw_command_str(kind, &command_container, index)
            .await?;

        Ok(PendingCommandAction { kind, raw_command })
    }

    async fn handle_command_by_index(
        &self,
        index: usize,
        decision: CommandDecision,
    ) -> Result<(), Error> {
        let pending_command = self.resolve_pending_command_action(index).await?;

        let action_button = wait_for_selector(
            &self.main_page,
            pending_command.kind.action_button_selector(decision),
            Duration::from_millis(COMMAND_ACTION_TIMEOUT_MS),
        )
        .await?;
        action_button.click().await?;

        // Click confirm button
        if matches!(
            (pending_command.kind, decision),
            (CommandCardKind::Delete, CommandDecision::Allow)
        ) {
            self.confirm_delete_command().await?;
        }

        println!(
            "{} Command: {}",
            decision.log_label(),
            pending_command.raw_command
        );

        sleep(Duration::from_millis(500)).await;

        Ok(())
    }

    pub async fn allow_command_by_index(&self, index: usize) -> Result<(), Error> {
        self.handle_command_by_index(index, CommandDecision::Allow)
            .await
    }

    pub async fn reject_command_by_index(&self, index: usize) -> Result<(), Error> {
        self.handle_command_by_index(index, CommandDecision::Reject)
            .await
    }

    async fn detect_hitl_prompt_kind(&self) -> Result<Option<HitlPromptKind>, Error> {
        // use `:?` to check string literal in debug
        let question_selector = format!("{QUESTION_CARD_SELECTOR:?}");
        let command_selector = format!("{COMMAND_CARD_SELECTOR:?}");

        let raw_kind: Option<String> = self
            .main_page
            .evaluate(format!(
                r#"
        (() => {{
            if (document.querySelector({question_selector})) {{
                return "question";
            }}

            if (document.querySelector({command_selector})) {{
                return "command";
            }}

            return null;
        }})()
        "#
            ))
            .await?
            .into_value()?;

        Ok(match raw_kind.as_deref() {
            Some("question") => Some(HitlPromptKind::Questionnaire),
            Some("command") => Some(HitlPromptKind::Command),
            _ => None,
        })
    }

    async fn wait_for_hitl_prompt_kind(&self) -> Result<HitlPromptKind, Error> {
        let timeout = Duration::from_millis(HITL_PROMPT_TIMEOUT_MS); // 10 secs
        let start = Instant::now();

        // wait until the prompt card to display
        while start.elapsed() < timeout {
            if let Some(kind) = self.detect_hitl_prompt_kind().await? {
                return Ok(kind);
            }

            sleep(Duration::from_millis(COMMAND_ACTION_POLL_INTERVAL_MS)).await;
        }

        Err(Error::msg(
            "Cannot find a pending HITL command card or question card.",
        ))
    }

    async fn handle_configured_command_by_index(&self, index: usize) -> Result<(), Error> {
        match self.config.command_strategy {
            CommandStrategy::Allow => self.allow_command_by_index(index).await,
            CommandStrategy::Deny => self.reject_command_by_index(index).await,
            CommandStrategy::LLM => Err(Error::msg(
                "command_strategy = \"llm\" is not implemented yet. Use \"allow\" or \"deny\".",
            )),
        }
    }

    async fn answer_questionnaire_by_index(&self, index: usize) -> Result<(), Error> {
        self.select_task_by_index(index).await?;

        // QUESTION_MAX_STEPS is a safety valve for infinite loop
        for step in 0..QUESTION_MAX_STEPS {
            let Some(question) = self.extract_questionnaire().await? else {
                // No question card found, throw an error
                if step == 0 {
                    bail!("Cannot find a pending questionnaire card.");
                }

                return Ok(());
            };

            let signature = Self::questionnaire_signature(&question);
            self.answer_current_questionnaire(&question).await?;

            // 给页面一点时间完成切换动画 / DOM 更新，减少立刻检查造成的假阴性。
            sleep(Duration::from_millis(1000)).await;

            if matches!(self.config.question_strategy, QuestionStrategy::Skip) {
                return Ok(());
            }

            self.wait_for_question_transition(&signature).await?;
        }

        Err(Error::msg(format!(
            "Questionnaire did not finish within {} steps.",
            QUESTION_MAX_STEPS
        )))
    }

    async fn extract_questionnaire(&self) -> Result<Option<Questionnaire>, Error> {
        let card_selector = format!("{QUESTION_CARD_SELECTOR:?}");
        let title_selector = format!("{QUESTION_TITLE_SELECTOR:?}");
        let option_content_selector = format!("{QUESTION_OPTION_CONTENT_SELECTOR:?}");
        let option_label_selector = format!("{QUESTION_OPTION_LABEL_SELECTOR:?}");
        let option_description_selector = format!("{QUESTION_OPTION_DESCRIPTION_SELECTOR:?}");
        let context_selector = format!("{QUESTION_CONTEXT_SELECTOR:?}");
        let multiple_choice_selector = format!("{QUESTION_MULTIPLE_CHOICE_SELECTOR:?}");
        let textarea_selector = format!("{QUESTION_TEXTAREA_SELECTOR:?}");

        Ok(self
            .main_page
            .evaluate(format!(
                r#"
        (() => {{
            // only read pending card
            const card = document.querySelector({card_selector});
            if (!(card instanceof HTMLElement)) {{
                return null;
            }}

            const questionTitle = card.querySelector({title_selector});
            const questionText = (questionTitle?.innerText ?? questionTitle?.textContent ?? '').trim();
            if (!questionText) {{
                return null;
            }}

            const textInput = card.querySelector({textarea_selector});
            const hasTextInput = textInput instanceof HTMLTextAreaElement;
            const textInputPlaceholder = hasTextInput
                ? (textInput.getAttribute('placeholder') ?? '').trim()
                : null;

            // filter objective options (No input action required)
            // - exclude `is-others`
            // - exclude `input`
            const options = Array.from(card.querySelectorAll({option_content_selector}))
                .flatMap((item) => {{
                    const container = item.closest('div[class*="icd-checkbox-item-container"]');
                    if ((container?.className ?? '').toString().includes('is-others')) {{
                        return [];
                    }}

                    const title = item.querySelector({option_label_selector});
                    const titleText = (title?.innerText ?? title?.textContent ?? '').trim();
                    if (!titleText || titleText === '其他') {{
                        return [];
                    }}

                    const description = item.querySelector({option_description_selector});
                    const descriptionText = (description?.innerText ?? description?.textContent ?? '').trim();

                    return [{{
                        title: titleText,
                        description: descriptionText,
                    }}];
                }});

            // A valid question card should be:
            // 1. options
            // 2. No options, but got textarea
            if (options.length === 0 && !hasTextInput) {{
                return null;
            }}

            const contextElements = Array.from(document.querySelectorAll({context_selector}));
            const latestContext = contextElements.at(-1);
            const context = (latestContext?.innerText ?? latestContext?.textContent ?? '').trim();
            const isMultiple = !!card.querySelector({multiple_choice_selector});

            return {{
                context,
                question: questionText,
                options,
                is_multiple: isMultiple,
                has_text_input: hasTextInput,
                text_input_placeholder: textInputPlaceholder,
            }};
        }})()
        "#
            ))
            .await?
            .into_value()?)
    }

    /// WaitingForHITL handler
    pub async fn handle_human_in_loop_by_index(&self, index: usize) -> Result<(), Error> {
        self.select_task_by_index(index).await?;

        match self.wait_for_hitl_prompt_kind().await? {
            HitlPromptKind::Command => self.handle_configured_command_by_index(index).await,
            HitlPromptKind::Questionnaire => self.answer_questionnaire_by_index(index).await,
        }
    }

    /// Generate a signature to check if the question is correctly handled
    /// 1. question title may change sometime
    /// 2. a question may contains multi options and perhaps a textarea
    /// 3. encode question, option titles and textarea
    fn questionnaire_signature(question: &Questionnaire) -> String {
        let option_titles = question
            .options
            .iter()
            .map(|option| option.title.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            "{}\n{}\n{}\n{}",
            question.question,
            option_titles,
            question.has_text_input,
            question
                .text_input_placeholder
                .as_deref()
                .unwrap_or_default()
        )
    }

    fn normalize_question_selection(
        question: &Questionnaire,
        mut indices: Vec<usize>,
    ) -> Result<Vec<usize>, Error> {
        indices.retain(|index| *index < question.options.len());
        indices.sort_unstable();
        indices.dedup();

        if indices.is_empty() {
            bail!(
                "No valid option index was selected for question: {}",
                question.question
            );
        }

        if !question.is_multiple && indices.len() > 1 {
            indices.truncate(1);
        }

        Ok(indices)
    }

    fn choose_random_question_option(&self, question: &Questionnaire) -> Result<Vec<usize>, Error> {
        if question.options.is_empty() {
            bail!("Cannot auto-answer a question without options.");
        }

        // only return an option
        Ok(vec![rand::random_range(0..question.options.len())])
    }

    fn build_llm_client(&self) -> Client<OpenAIConfig> {
        let mut openai_config = OpenAIConfig::new();
        let model_config = &self.config.model;

        if !model_config.api_key.trim().is_empty() {
            openai_config = openai_config.with_api_key(model_config.api_key.trim());
        }

        if !model_config.base_url.trim().is_empty() {
            openai_config = openai_config.with_api_base(model_config.base_url.trim());
        }

        Client::with_config(openai_config)
    }

    fn build_question_llm_prompt(question: &Questionnaire) -> Result<String, Error> {
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "task_context": question.context,
            "question": question.question,
            "is_multiple": question.is_multiple,
            "options": question
                .options
                .iter()
                .enumerate()
                .map(|(index, option)| {
                    serde_json::json!({
                        "index": index,
                        "title": option.title,
                        "description": option.description,
                    })
                })
                .collect::<Vec<_>>(),
            "response_schema": {
                "option_indices": "array of zero-based option indexes to choose"
            }
        }))?)
    }

    /// JSON format fence for LLM response
    fn parse_llm_question_selection(content: &str) -> Result<LlmQuestionSelection, Error> {
        let trimmed = content.trim();

        if let Ok(selection) = serde_json::from_str(trimmed) {
            return Ok(selection);
        }

        let fenced = trimmed
            .trim_start_matches("```json")
            .trim_start_matches("```")
            .trim_end_matches("```")
            .trim();

        Ok(serde_json::from_str(fenced)?)
    }

    async fn choose_question_options_with_llm(
        &self,
        question: &Questionnaire,
    ) -> Result<Vec<usize>, Error> {
        let model_name = self.config.model.model_name.trim();
        if model_name.is_empty() {
            bail!("model.model_name cannot be empty when question_strategy = \"llm\".");
        }

        let request = CreateChatCompletionRequestArgs::default()
            .model(model_name)
            .messages([
                ChatCompletionRequestSystemMessageArgs::default()
                    .content(
                        "You answer IDE assistant questionnaire cards. Return only JSON in the shape {\"option_indices\":[0]}. Use zero-based option indexes from the provided options. For single-choice questions, return exactly one index. Never invent options.",
                    )
                    .build()?
                    .into(),
                ChatCompletionRequestUserMessageArgs::default()
                    .content(Self::build_question_llm_prompt(question)?)
                    .build()?
                    .into(),
            ])
            .response_format(ResponseFormat::JsonObject)
            .temperature(0.0_f32)
            .max_completion_tokens(128_u32)
            .build()?;

        let response = self.build_llm_client().chat().create(request).await?;
        let content = response
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .ok_or_else(|| Error::msg("LLM did not return a text answer for the question."))?;
        let selection = Self::parse_llm_question_selection(content)?;

        Self::normalize_question_selection(question, selection.option_indices)
    }

    async fn choose_question_option_indices(
        &self,
        question: &Questionnaire,
    ) -> Result<Vec<usize>, Error> {
        // decision maker on how to handle QA card
        match self.config.question_strategy {
            QuestionStrategy::Skip => Ok(Vec::new()),
            QuestionStrategy::Auto => self.choose_random_question_option(question),
            QuestionStrategy::LLM => self.choose_question_options_with_llm(question).await,
        }
    }

    async fn click_question_cancel_action(&self) -> Result<bool, Error> {
        let card_selector = format!("{QUESTION_CARD_SELECTOR:?}");
        let action_bar_selector = format!("{QUESTION_ACTION_BAR_SELECTOR:?}");
        let cancel_text = format!("{QUESTION_CANCEL_BUTTON_TEXT:?}");

        Ok(self
            .main_page
            .evaluate(format!(
                r#"
        (() => {{
            const card = document.querySelector({card_selector});
            const actionBar = card?.querySelector({action_bar_selector});
            if (!(actionBar instanceof HTMLElement)) {{
                return false;
            }}

            const buttons = Array.from(actionBar.querySelectorAll('button'));
            const target = buttons.find((button) => {{
                const text = (button.textContent ?? button.innerText ?? '').trim();
                return text === {cancel_text};
            }}) ?? buttons.find((button) => (button.className ?? '').toString().includes('icd-btn-secondary'));

            if (!(target instanceof HTMLElement)) {{
                return false;
            }}

            target.click();
            return true;
        }})()
        "#
            ))
            .await?
            .into_value()?)
    }

    async fn click_question_option_by_index(&self, index: usize) -> Result<(), Error> {
        let card_selector = format!("{QUESTION_CARD_SELECTOR:?}");
        let option_content_selector = format!("{QUESTION_OPTION_CONTENT_SELECTOR:?}");
        let option_label_selector = format!("{QUESTION_OPTION_LABEL_SELECTOR:?}");

        // choose option by its index
        let clicked: bool = self
            .main_page
            .evaluate(format!(
                r#"
        (() => {{
            const card = document.querySelector({card_selector});
            if (!(card instanceof HTMLElement)) {{
                return false;
            }}

            const options = Array.from(card.querySelectorAll({option_content_selector}))
                .filter((item) => {{
                    const container = item.closest('div[class*="icd-checkbox-item-container"]');
                    if ((container?.className ?? '').toString().includes('is-others')) {{
                        return false;
                    }}

                    const title = item.querySelector({option_label_selector});
                    const titleText = (title?.innerText ?? title?.textContent ?? '').trim();
                    return !!titleText && titleText !== '其他';
                }});

            const target = options[{index}];
            if (!(target instanceof HTMLElement)) {{
                return false;
            }}

            const clickable = target.closest('div[class*="icd-checkbox-item-container"]') ?? target;
            if (!(clickable instanceof HTMLElement)) {{
                return false;
            }}

            clickable.click();
            return true;
        }})()
        "#
            ))
            .await?
            .into_value()?;

        if !clicked {
            bail!("Cannot click question option by index: {}", index);
        }

        sleep(Duration::from_millis(100)).await;
        Ok(())
    }

    /// Click 'Next' and 'Submit' button
    async fn click_question_primary_action(&self) -> Result<bool, Error> {
        let card_selector = format!("{QUESTION_CARD_SELECTOR:?}");
        let action_bar_selector = format!("{QUESTION_ACTION_BAR_SELECTOR:?}");
        let cancel_text = format!("{QUESTION_CANCEL_BUTTON_TEXT:?}");

        Ok(self
            .main_page
            .evaluate(format!(
                r#"
        (() => {{
            const card = document.querySelector({card_selector});
            const actionBar = card?.querySelector({action_bar_selector});
            if (!(actionBar instanceof HTMLElement)) {{
                return false;
            }}

            const buttons = Array.from(actionBar.querySelectorAll('button'))
                .filter((button) => button.getAttribute('aria-disabled') !== 'true' && !button.disabled);
            const target = buttons.find((button) => (button.className ?? '').toString().includes('icd-btn-primary'))
                ?? buttons.find((button) => {{
                    const text = (button.textContent ?? button.innerText ?? '').trim();
                    return text && text !== {cancel_text};
                }});

            if (!(target instanceof HTMLElement)) {{
                return false;
            }}

            target.click();
            return true;
        }})()
        "#
            ))
            .await?
            .into_value()?)
    }

    async fn wait_for_question_transition(&self, previous_signature: &str) -> Result<(), Error> {
        let timeout = Duration::from_millis(QUESTION_TRANSITION_TIMEOUT_MS);
        let start = Instant::now();

        // Check if the current question finished:
        // - go to next question: the signature should change
        // - submit: question card container disappear from page, `extract_questionnaire` return None
        // - not working: the signature remains the same and timeout
        while start.elapsed() < timeout {
            let current = self.extract_questionnaire().await?;
            let current_signature = current.as_ref().map(Self::questionnaire_signature);

            if current_signature.as_deref() != Some(previous_signature) {
                return Ok(());
            }

            sleep(Duration::from_millis(QUESTION_TRANSITION_POLL_INTERVAL_MS)).await;
        }

        Err(Error::msg(
            "Questionnaire did not transition after selecting an option.",
        ))
    }

    async fn answer_current_questionnaire(&self, question: &Questionnaire) -> Result<(), Error> {
        if matches!(self.config.question_strategy, QuestionStrategy::Skip) {
            if !self.click_question_cancel_action().await? {
                bail!("Cannot find the questionnaire cancel button.");
            }

            println!("Skipped Question: {}", question.question);
            sleep(Duration::from_millis(300)).await;
            return Ok(());
        }

        // As some questions may only contain textarea, no options presented. For example:
        // Any extra info? (Optional).
        //  __________________________
        // |__________________________|
        //
        // For now we just skip them, because input text in the textarea will lead a lot of work and maybe bugs.
        if question.options.is_empty() && question.has_text_input {
            sleep(Duration::from_millis(200)).await;

            if !self.click_question_primary_action().await? {
                bail!("Cannot find the questionnaire primary action button.");
            }

            println!("Submitted Text Question Empty: {}", question.question);

            return Ok(());
        }

        // For general QA:
        // 1. decide which options should we choose
        // 2. click them sequentially
        // 3. click `Next` or `Submit`
        let selected_indices = self.choose_question_option_indices(question).await?;

        for index in &selected_indices {
            self.click_question_option_by_index(*index).await?;
        }

        sleep(Duration::from_millis(200)).await;

        if !self.click_question_primary_action().await? {
            bail!("Cannot find the questionnaire primary action button.");
        }

        let selected_titles = selected_indices
            .iter()
            .filter_map(|index| question.options.get(*index))
            .map(|option| option.title.as_str())
            .collect::<Vec<_>>()
            .join(", ");

        println!(
            "Answered Question: {} -> {}",
            question.question, selected_titles
        );

        Ok(())
    }

    pub async fn terminate_task_by_index(&self, index: usize) -> Result<(), Error> {
        let task = self.get_task_by_index(index).await?;
        match task.status {
            TraeTaskStatus::Running | TraeTaskStatus::WaitingForHITL => {
                self.click_element_by_selector("button[class*=chat-input-v2-send-button]")
                    .await
            }
            _ => Err(Error::msg(
                "You cannot terminate this task as it's not started yet.",
            )),
        }
    }

    pub async fn click_send_button_by_index(&self, index: usize) -> Result<(), Error> {
        let task = self.get_task_by_index(index).await?;
        match task.status {
            TraeTaskStatus::Finished | TraeTaskStatus::Interrupted | TraeTaskStatus::Idle => {
                self.click_element_by_selector("button[class*=chat-input-v2-send-button]")
                    .await
            }
            _ => Err(Error::msg(
                "You cannot click send button as this task is still running. Invoke `terminate` method first.",
            )),
        }
    }

    async fn get_task_by_index(&self, index: usize) -> Result<TraeTask, Error> {
        let latest_tasks = self.get_tasks().await?;

        let target = latest_tasks
            .get(index)
            .cloned()
            .ok_or_else(|| Error::msg(format!("Cannot find task by index: {}", index)))?;

        Ok(target)
    }

    pub async fn get_task_handle_by_index(
        &self,
        index: usize,
    ) -> Result<TraeTaskHandler<'_>, Error> {
        let task = self.get_task_by_index(index).await?;
        Ok(TraeTaskHandler::new(self, task))
    }

    /// Operations
    pub async fn select_task_by_index(&self, index: usize) -> Result<(), Error> {
        let _ = wait_for_selector(
            &self.main_page,
            "#solo-ai-sidebar-content div[class*=task-items-list]",
            Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
        )
        .await
        .expect("Cannot get task container.");

        let clicked: bool = self
            .main_page
            .evaluate(format!(
                r#"
                (() => {{
                    const items = Array.from(document.querySelectorAll(
                      '#solo-ai-sidebar-content div[class*="task-items-list"] div[class*="index-module__task-item___"]'
                    ));
                    const target = items[{index}];

                    if (!target) {{
                      return false;
                    }}

                    target.click();
                    return true;
                }})()
                "#
            ))
            .await?
            .into_value()?;

        if !clicked {
            return Err(Error::msg(format!(
                "Cannot find task element by index: {}",
                index
            )));
        }

        Ok(())
    }

    pub async fn type_content_to_chat_input(&self, content: &str) -> Result<(), Error> {
        self.focus_chat_input().await?;
        self.clear_chat_input().await?;
        self.insert_text_to_focused_input(content).await?;

        Ok(())
    }

    async fn is_interoperable(&self, index: usize) -> Result<(), Error> {
        let task = self.get_task_by_index(index).await?;

        match task.status {
            TraeTaskStatus::Finished | TraeTaskStatus::Interrupted => Ok(()),
            _ => {
                return Err(Error::msg(
                    "Actions can only be trigger under Finished/Interrupted status.",
                ));
            }
        }
    }

    pub async fn copy_task_summary_by_index(&self, index: usize) -> Result<(), Error> {
        // status guard
        self.is_interoperable(index).await?;

        let copy_summary_button = wait_for_selector(
            &self.main_page,
            "button[aria-label=复制全部]",
            Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
        )
        .await?;

        copy_summary_button.click().await?;
        Ok(())
    }

    pub async fn feedback_task_by_index(
        &self,
        index: usize,
        feedback: TraeSoloTaskFeedback,
    ) -> Result<(), Error> {
        // status guard
        self.is_interoperable(index).await?;

        match feedback {
            TraeSoloTaskFeedback::Good => {
                let feedback_good_button = wait_for_selector(
                    &self.main_page,
                    "button[aria-label=赞]",
                    Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
                )
                .await?;

                feedback_good_button.click().await?;
            }
            TraeSoloTaskFeedback::Bad => {
                let feedback_bad_button = wait_for_selector(
                    &self.main_page,
                    "button[aria-label=踩]",
                    Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
                )
                .await?;

                feedback_bad_button.click().await?;
            }
        }

        Ok(())
    }

    pub async fn retry_task_by_index(&self, index: usize) -> Result<(), Error> {
        // status guard
        self.is_interoperable(index).await?;

        let retry_button = wait_for_selector(
            &self.main_page,
            "button[aria-label=重试]",
            Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
        )
        .await?;

        retry_button.click().await?;
        Ok(())
    }

    pub async fn cached_tasks(&self) -> Vec<TraeTask> {
        self.tasks.read().await.clone()
    }

    pub async fn run_task_sync_loop(
        &self,
        interval: Duration,
        workflow: TaskWorkflow,
        initial_policy: InitialTaskPolicy,
        mut shutdown_rx: Receiver<bool>,
    ) {
        match self.bootstrap_tasks_with_events(initial_policy).await {
            Ok(events) => self.dispatch_task_events(&workflow, events).await,
            Err(err) => eprintln!("bootstrap_tasks_with_events failed: {err:?}"),
        }

        let mut ticker = time::interval(interval);
        ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    match self.refresh_tasks_with_events().await {
                        Ok(events) => {
                            self.dispatch_task_events(&workflow, events).await;
                        }
                        Err(err) => eprintln!("refresh_tasks_with_events failed: {err:?}"),
                    }
                }
                changed = shutdown_rx.changed() => {
                    match changed {
                        Ok(_) if *shutdown_rx.borrow() => break,
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
            }
        }
    }
}
