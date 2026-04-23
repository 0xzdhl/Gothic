use crate::config::Config;
use crate::consts::*;
use crate::trae::{
    NewTraeTask, TraeTask, TraeTaskStatus, diff_task_status_changes, handle_task_status_change,
    types::*,
};
use crate::utils::{normalize_executable_path_for_cdp, wait_for_selector};
use anyhow::{Error, Result};
use chromiumoxide::Element;
use chromiumoxide::cdp::browser_protocol::input::InsertTextParams;
use chromiumoxide::{Browser, Page, cdp::browser_protocol::target::TargetInfo};
use serde::Deserialize;
use tokio::sync::RwLock;
use tokio::sync::watch::Receiver;
use tokio::time::{self, Duration, Instant, sleep};

// TODO: Refactor
const CHAT_INPUT_SELECTOR: &str =
    "#agent-chat-view div.chat-input-wrapper div.chat-input-v2-input-box-editable";

const COMMAND_CARD_SELECTOR: &str = "div[class*=icd-run-command-card-v2-command-shell],div[class*=icd-delete-files-command-card-v2]";
const DELETE_COMMAND_CARD_CLASS: &str = "icd-delete-files-command-card-v2";
const DELETE_COMMAND_TEXT_SELECTOR: &str = "div[class*=icd-delete-files-command-card-v2-cwd]";
const RUN_COMMAND_TEXT_SELECTOR: &str = "div[class*=icd-run-command-card-v2-command-shell]";
const DELETE_COMMAND_ALLOW_SELECTOR: &str =
    "button[class*=icd-delete-files-command-card-v2-actions-delete]";
const DELETE_COMMAND_REJECT_SELECTOR: &str =
    "button[class*=icd-delete-files-command-card-v2-actions-cancel]";
const RUN_COMMAND_ALLOW_SELECTOR: &str = "button[class*=icd-run-command-card-v2-actions-btn-run]";
const RUN_COMMAND_REJECT_SELECTOR: &str =
    "button[class*=icd-run-command-card-v2-actions-btn-cancel]";
const DELETE_CONFIRM_POPOVER_SELECTOR: &str = "div[class*=confirm-popover-body]";
const DELETE_CONFIRM_BUTTON_TEXT: &str = "\u{786E}\u{8BA4}";
const COMMAND_ACTION_TIMEOUT_MS: u64 = 1000 * 30;
const COMMAND_ACTION_POLL_INTERVAL_MS: u64 = 300;

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
