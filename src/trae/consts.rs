/// Trae editor common elements
pub const TRAE_SOLO_MODE_TEXT_LABEL: &'static str = r"切换至 SOLO 模式(Ctrl+Alt+\)";
pub const TRAE_IDE_MODE_TEXT_LABEL: &'static str = r"切换至 IDE 模式(Ctrl+Alt+\)";
pub const TRAE_SOLO_TASK_INTERRUPTED_LABEL: &'static str = "任务中断";
pub const TRAE_SOLO_TASK_RUNNING_LABEL: &'static str = "进行中";
pub const TRAE_SOLO_TASK_FINISHED_LABEL: &'static str = "任务完成";
pub const TRAE_SOLO_TASK_WAITING_FOR_HITL_LABEL: &'static str = "等待操作";

/// Global selector timeout
pub const DEFAULT_SELECTOR_TIMEOUT: u64 = 1000 * 30;

/// Chat input box, for interaction
pub const CHAT_INPUT_SELECTOR: &str =
    "#agent-chat-view div.chat-input-wrapper div.chat-input-v2-input-box-editable";

/// WaitingForHITL - Command card container and operations
pub const COMMAND_CARD_SELECTOR: &str = "div[class*=icd-run-command-card-v2-command-shell],div[class*=icd-delete-files-command-card-v2]";
pub const DELETE_COMMAND_CARD_CLASS: &str = "icd-delete-files-command-card-v2";
pub const DELETE_COMMAND_TEXT_SELECTOR: &str = "div[class*=icd-delete-files-command-card-v2-cwd]";
pub const RUN_COMMAND_TEXT_SELECTOR: &str = "div[class*=icd-run-command-card-v2-command-shell]";
pub const DELETE_COMMAND_ALLOW_SELECTOR: &str =
    "button[class*=icd-delete-files-command-card-v2-actions-delete]";
pub const DELETE_COMMAND_REJECT_SELECTOR: &str =
    "button[class*=icd-delete-files-command-card-v2-actions-cancel]";
pub const RUN_COMMAND_ALLOW_SELECTOR: &str =
    "button[class*=icd-run-command-card-v2-actions-btn-run]";
pub const RUN_COMMAND_REJECT_SELECTOR: &str =
    "button[class*=icd-run-command-card-v2-actions-btn-cancel]";
pub const DELETE_CONFIRM_POPOVER_SELECTOR: &str = "div[class*=confirm-popover-body]";
pub const DELETE_CONFIRM_BUTTON_TEXT: &str = "\u{786E}\u{8BA4}";
pub const COMMAND_ACTION_TIMEOUT_MS: u64 = 1000 * 30;
pub const COMMAND_ACTION_POLL_INTERVAL_MS: u64 = 300;

/// WaitingForHITL - Question card and operations
pub const HITL_PROMPT_TIMEOUT_MS: u64 = 1000 * 10;
pub const QUESTION_CARD_SELECTOR: &str =
    "div[class*=ask-user-question-card-container][class*=ask-user-question-card-status-pending]";
pub const QUESTION_TITLE_SELECTOR: &str = "div[class*=ask-user-question-content-title-section]";
pub const QUESTION_OPTION_CONTENT_SELECTOR: &str = "div[class*=icd-checkbox-item-content]";
pub const QUESTION_OPTION_LABEL_SELECTOR: &str = "div[class*=icd-checkbox-item-label]";
pub const QUESTION_OPTION_DESCRIPTION_SELECTOR: &str = "div[class*=icd-checkbox-item-description]";
pub const QUESTION_ACTION_BAR_SELECTOR: &str = "div[class*=icd-action-bar-right]";
pub const QUESTION_CONTEXT_SELECTOR: &str = "div[class*=user-chat-line]";
pub const QUESTION_MULTIPLE_CHOICE_SELECTOR: &str =
    "div[class*=ask-user-multiple-choice-container]";
pub const QUESTION_TEXTAREA_SELECTOR: &str =
    "textarea[class*=ask-user-question-textarea-input-textarea]";
pub const QUESTION_CANCEL_BUTTON_TEXT: &str = "\u{53D6}\u{6D88}";
pub const QUESTION_MAX_STEPS: usize = 20;
pub const QUESTION_TRANSITION_TIMEOUT_MS: u64 = 1000 * 5;
pub const QUESTION_TRANSITION_POLL_INTERVAL_MS: u64 = 200;
