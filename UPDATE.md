可以，而且我建议你**把“任务快照”与“任务动作”彻底拆开**。

你现在的设计偏重，是因为把 `TraeSoloTask` 同时当成了：

1. **任务状态数据**
2. **任务行为入口**
3. **对 editor 的借用句柄**

这三件事混在一起了。尤其是 `TraeSoloTask<'a>` 里持有 `&'a TraeEditor`，而 `TraeEditor::get_tasks(&self)` 又返回 `Vec<TraeSoloTask<'_>>`，这使它很难再“存回 editor 里作为缓存”，因为这会变成自引用式设计，在 Rust 里非常别扭。你当前大量 `PhantomData + trait state + enum 包装` 也说明模型已经超过了“轮询 sidebar 任务列表”这个需求本身。 

你现在真正需要的是这套结构：

* **轻量任务快照**：只描述 sidebar 里当前看到的任务
* **Editor 持有任务缓存**
* **定时 refresh_tasks()**
* **外部随时读取 cached_tasks()**
* 动作类 API 另外放，不要塞进缓存对象里

## 我建议你改成这样

### 1. 把状态类型收轻

你现在这套：

* `Interrupted`
* `Running`
* `WaitingForHITL`
* `Finished`
* `Idle`
* `TaskState`
* `Action`
* `TraeSoloTaskInner<'a, S>`

对于“轮询 + 刷新列表”来说太重了。

直接改成：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraeTaskStatus {
    Idle,
    Running,
    Interrupted,
    WaitingForHITL,
    Finished,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraeTask {
    pub title: String,
    pub status: TraeTaskStatus,
    pub selected: bool,
}
```

这就够了。

因为 sidebar 里的任务，本质上只是一个 **UI snapshot**，不是“可编译期约束状态迁移的领域对象”。

---

### 2. Editor 里直接存缓存列表

```rust
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct TraeEditor {
    pub(crate) main_page: Page,
    pub(crate) target: TargetInfo,
    pub(crate) prebuilt_agent: TraeEditorPrebuiltSoloAgent,
    pub(crate) mode: TraeEditorMode,

    pub(crate) tasks: RwLock<Vec<TraeTask>>,
}
```

初始化时：

```rust
return TraeEditor {
    target: main_target,
    main_page,
    prebuilt_agent: TraeEditorPrebuiltSoloAgent::Coder,
    mode: current_mode,
    tasks: RwLock::new(Vec::new()),
};
```

这样 editor 自己就是任务列表的唯一持有者。

---

### 3. `get_tasks` 改名为 `fetch_tasks_from_ui`

因为它做的是“从页面抓取最新状态”，不是“获取缓存”。

```rust
impl TraeEditor {
    pub async fn fetch_tasks_from_ui(&self) -> Result<Vec<TraeTask>, Error> {
        if self.mode != TraeEditorMode::SOLO {
            return Err(Error::msg(
                "Cannot get tasks under IDE mode, please switch to SOLO mode.",
            ));
        }

        let task_container = self
            .main_page
            .find_element("#solo-ai-sidebar-content div[class*=task-items-list]")
            .await?;

        let task_items = task_container
            .find_elements(r#"div[class*="index-module__task-item___"]"#)
            .await?;

        let mut tasks = Vec::with_capacity(task_items.len());

        for item in task_items {
            let class_name = item
                .attribute("class")
                .await?
                .unwrap_or_default();

            let selected = class_name.contains("selected");

            let raw_task_state = item
                .find_element(r#"div[class*="task-type-wrap"]"#)
                .await?
                .inner_html()
                .await?
                .unwrap_or_default();

            let title = item
                .find_element(r#"span[class*="task-title"]"#)
                .await?
                .inner_html()
                .await?
                .unwrap_or_default();

            let status = match raw_task_state.as_str() {
                TRAE_SOLO_TASK_INTERRUPTED_LABEL => TraeTaskStatus::Interrupted,
                TRAE_SOLO_TASK_RUNNING_LABEL => TraeTaskStatus::Running,
                TRAE_SOLO_TASK_WAITING_FOR_HITL_LABEL => TraeTaskStatus::WaitingForHITL,
                TRAE_SOLO_TASK_FINISHED_LABEL => TraeTaskStatus::Finished,
                TRAE_SOLO_TASK_IDLE_LABEL => TraeTaskStatus::Idle,
                _ => TraeTaskStatus::Unknown,
            };

            tasks.push(TraeTask {
                title,
                status,
                selected,
            });
        }

        Ok(tasks)
    }
}
```

这里顺手帮你指出两个实际问题：

* 你现在的 `find_elements(r#"div[class*="index-module__task-item___"#)` 这个 selector 少了结尾 `"]`，是坏的。
* `find_element("div[class*=task-type-wrap")` 也少了 `]`。
  这两个地方就算架构不改，后面也容易直接查不到元素。

---

### 4. 增加 `refresh_tasks()`

```rust
impl TraeEditor {
    pub async fn refresh_tasks(&self) -> Result<Vec<TraeTask>, Error> {
        let latest = self.fetch_tasks_from_ui().await?;

        let mut guard = self.tasks.write().await;
        *guard = latest.clone();

        Ok(latest)
    }

    pub async fn cached_tasks(&self) -> Vec<TraeTask> {
        self.tasks.read().await.clone()
    }
}
```

这样职责就很清楚：

* `fetch_tasks_from_ui()`：抓页面
* `refresh_tasks()`：抓页面并更新缓存
* `cached_tasks()`：只读缓存

---

### 5. 增加定时同步循环

如果你想持续刷新：

```rust
use tokio::time::{self, Duration, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

impl TraeEditor {
    pub async fn run_task_sync_loop(
        &self,
        interval: Duration,
        cancel_token: CancellationToken,
    ) {
        let mut ticker = time::interval(interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    break;
                }
                _ = ticker.tick() => {
                    if let Err(err) = self.refresh_tasks().await {
                        eprintln!("refresh_tasks failed: {err:?}");
                    }
                }
            }
        }
    }
}
```

调用方式：

```rust
let cancel_token = CancellationToken::new();

// 先手动刷新一次，避免缓存为空
editor.refresh_tasks().await?;

// 然后启动同步循环
tokio::spawn({
    let cancel = cancel_token.clone();
    async move {
        editor.run_task_sync_loop(Duration::from_secs(2), cancel).await;
    }
});
```

不过这里有一个现实点要提醒你：

如果 `chromiumoxide::Page` 相关对象不是 `Send`，那 `tokio::spawn` 可能过不了。那就不要在 `TraeEditor` 内部强行 `spawn`，改成：

* 在当前 async 上下文里直接 `await run_task_sync_loop(...)`
* 或者用 `spawn_local`
* 或者外层自己安排调度

也就是说，**最好让 editor 提供 loop 方法，不要强绑定内部 spawn 策略**。

---

## 最重要的一点：把“动作对象”单独做，不要复用任务缓存对象

你现在的 `TraeSoloTask` 里面有：

* `execute`
* `optimize_prompt`
* `copy_task_summary`
* `retry`

这类 API 更像“命令句柄”，不适合跟缓存列表共用一个结构。

我建议拆成两层：

### A. 缓存层

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraeTask {
    pub title: String,
    pub status: TraeTaskStatus,
    pub selected: bool,
}
```

### B. 动作层

```rust
pub struct NewTraeTask<'a> {
    editor: &'a TraeEditor,
    prompt: String,
}

impl<'a> NewTraeTask<'a> {
    pub fn new(editor: &'a TraeEditor, prompt: String) -> Self {
        Self { editor, prompt }
    }

    pub async fn execute(&self) -> Result<(), Error> {
        // 你现有 execute 逻辑搬过来
        Ok(())
    }

    pub async fn optimize_prompt(&self) -> Result<(), Error> {
        Ok(())
    }
}
```

中断/完成后的操作也做成 editor 方法或 action handle：

```rust
impl TraeEditor {
    pub async fn retry_selected_task(&self) -> Result<(), Error> {
        todo!()
    }

    pub async fn copy_selected_task_summary(&self) -> Result<String, Error> {
        todo!()
    }
}
```

这样你就不会出现“列表里的每个 task 都带着 editor 借用和一堆行为”的负担。

---

## 一个更完整、也更稳的最终形态

### `types.rs`

```rust
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraeTaskStatus {
    Idle,
    Running,
    Interrupted,
    WaitingForHITL,
    Finished,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraeTask {
    pub title: String,
    pub status: TraeTaskStatus,
    pub selected: bool,
}

pub enum TraeSoloTaskFeedback {
    Good,
    Bad,
}
```

### `editor.rs`

```rust
use tokio::sync::RwLock;

#[derive(Debug)]
pub struct TraeEditor {
    pub(crate) main_page: Page,
    pub(crate) target: TargetInfo,
    pub(crate) prebuilt_agent: TraeEditorPrebuiltSoloAgent,
    pub(crate) mode: TraeEditorMode,
    pub(crate) tasks: RwLock<Vec<TraeTask>>,
}
```

### `editor task methods`

```rust
impl TraeEditor {
    pub async fn create_new_task(&self, prompt: impl Into<String>) -> NewTraeTask<'_> {
        NewTraeTask::new(self, prompt.into())
    }

    pub async fn refresh_tasks(&self) -> Result<Vec<TraeTask>, Error> {
        let latest = self.fetch_tasks_from_ui().await?;
        *self.tasks.write().await = latest.clone();
        Ok(latest)
    }

    pub async fn cached_tasks(&self) -> Vec<TraeTask> {
        self.tasks.read().await.clone()
    }
}
```

### `task_actions.rs`

```rust
pub struct NewTraeTask<'a> {
    editor: &'a TraeEditor,
    prompt: String,
}

impl<'a> NewTraeTask<'a> {
    pub fn new(editor: &'a TraeEditor, prompt: String) -> Self {
        Self { editor, prompt }
    }

    pub async fn execute(&self) -> Result<(), Error> {
        // 你的 execute 逻辑
        Ok(())
    }
}
```

---

## 你当前代码里，我还会顺手修这几个点

### 1. `optimize_prompt()` 调错了

你这里：

```rust
pub async fn optimize_prompt(&self) -> Result<(), Error> {
    match self {
        TraeSoloTask::Idle(t) => t.execute().await,
        _ => Err(Error::msg(
            "`optimize_prompt` can only be invoked when state is idle",
        )),
    }
}
```

你调用的是 `t.execute()`，不是 `t.optimize_prompt()`。这是明显 bug。

---

### 2. `get_current_editor_mode()` 的返回值看起来反了

你这里：

```rust
if mode_description.eq(TRAE_SOLO_MODE_TEXT_LABEL) {
    Ok(TraeEditorMode::IDE)
} else if mode_description.eq(TRAE_IDE_MODE_TEXT_LABEL) {
    Ok(TraeEditorMode::SOLO)
}
```

如果常量名和字面含义一致，这里应该是写反了。除非你的常量本身就是反向命名，否则这是另一个明显风险点。

---

### 3. 不要在抓任务时构造“带借用的任务对象”

这是你设计越来越沉重的核心原因。
`Vec<TraeSoloTask<'_>>` 看起来“类型安全”，但它会严重限制缓存、广播、TUI 状态同步。

对 TUI 来说，更适合的是：

* `Vec<TraeTask>` 存状态
* `watch::Receiver<Vec<TraeTask>>` 或 `RwLock<Vec<TraeTask>>` 给界面读
* 动作单独调用 editor API

---

## 我对你这个场景的最终建议

**保留状态机，只在“创建任务流程”里用。**
**缓存任务列表时，绝对不要用状态机对象。**

也就是：

* `NewTraeTask<'a>`：可以保留一些流程型方法
* `TraeTask`：纯数据快照
* `TraeEditor`：持有 `RwLock<Vec<TraeTask>>`
* `refresh_tasks()`：抓最新 sidebar 并覆盖缓存
* `run_task_sync_loop()`：定时刷新

这是最贴合你现在需求的解法，也最容易接进后面的 TUI。

你这次想要的不是“更强类型的 task 状态迁移系统”，而是一个**可轮询、可缓存、可展示、可扩展**的 editor 内部状态模型。

我可以下一条直接帮你把这三份文件重写成一个可编译版本：`types.rs`、`editor.rs`、`task_actions.rs`。


意思是：

**`TraeEditor` 只负责提供“持续刷新任务”的异步逻辑本身，至于这个逻辑是在当前任务里直接 `await`，还是被 `tokio::spawn` 到后台，由调用方决定。**

也就是把这两件事分开：

1. **业务逻辑**：每隔 N 秒刷新一次 tasks
2. **调度策略**：这个循环在哪跑、怎么跑、何时停止

---

### 你不应该这样写

```rust
impl TraeEditor {
    pub fn start_task_loop(&self) {
        tokio::spawn(async move {
            loop {
                let _ = self.refresh_tasks().await;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }
}
```

这个问题在于：

* `TraeEditor` 被迫依赖 `tokio::spawn`
* 调用方无法控制生命周期
* 不方便传取消信号
* `self` / `Page` / `Browser` 相关对象可能 **不是 `Send`**
* 后面你做 TUI 时，可能想在主事件循环里统一调度，而不是随便起后台任务

---

### 更好的写法是这样

```rust
impl TraeEditor {
    pub async fn run_task_sync_loop(
        &self,
        interval: Duration,
        cancel: CancellationToken,
    ) {
        let mut ticker = tokio::time::interval(interval);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = ticker.tick() => {
                    let _ = self.refresh_tasks().await;
                }
            }
        }
    }
}
```

这里 `editor` 只提供一个“loop 方法”，表示：

> “我会一直刷新 tasks，直到你让我停。”

但是它**不决定**自己是不是后台运行。

---

## 调用方就有选择权了

### 方式 1：直接在当前任务里跑

```rust
editor.run_task_sync_loop(Duration::from_secs(2), cancel_token).await;
```

适合：

* 当前就是一个专门的同步任务
* 不需要另起后台任务

---

### 方式 2：外部自己 `spawn`

```rust
tokio::spawn(async move {
    editor.run_task_sync_loop(Duration::from_secs(2), cancel_token).await;
});
```

适合：

* 你明确知道这里应该后台运行
* 你能保证捕获进去的对象满足 `Send + 'static` 等要求

---

### 方式 3：TUI 主循环里手动 tick

以后你做 OpenTUI 时，甚至可能根本不用 `spawn`，而是在 UI 主循环里定期调用：

```rust
if should_refresh {
    editor.refresh_tasks().await?;
}
```

这时候如果 `TraeEditor` 自己偷偷 `spawn`，反而不灵活。

---

## 核心思想就一句话

**Editor 提供能力，不替调用方做调度决策。**

它负责回答：

* 怎么刷新任务
* 怎么持续刷新
* 什么时候退出循环

但不负责强行决定：

* 一定要后台跑
* 一定要用 `tokio::spawn`
* 一定要自己管理线程/任务生命周期

---

## 为什么这对你特别重要

因为你的系统后面还有：

* TUI
* gRPC
* 浏览器/CDP连接
* 可能的多模块协作

这类系统里，**调度权最好集中在更外层**，例如：

* app runtime
* supervisor
* tui event loop
* service layer

而不是散落在 `TraeEditor` 这种底层对象里。

---

## 一个很形象的理解

* `refresh_tasks()` = “刷新一次”
* `run_task_sync_loop()` = “按规则持续刷新”
* `tokio::spawn(...)` = “安排谁去跑这件事”

前两者应该属于 `TraeEditor`。
最后那个“安排谁去跑”，最好留给外层。

---

你的场景里，我建议分层：

* `TraeEditor`: `refresh_tasks()`、`run_task_sync_loop()`
* `App` 或 `Supervisor`: 决定是否 `spawn`
* `TUI`: 只读 editor 里的缓存任务列表

我也可以下一条直接给你画一个这套结构的 Rust 模块图。

//=====================

下面这版是按你现在的目标重构的：

* `types.rs`：只保留轻量类型
* `task.rs`：只保留“新建任务”的动作句柄
* `editor.rs`：内部缓存 `Vec<TraeTask>`，并提供 `run_task_sync_loop`
* `loop` 只由 `editor` 提供，不在内部强行 `spawn`，由外层自己决定怎么跑

这正是为了解决你当前 `task.rs` 里“任务对象借用 editor + 状态机过重”和 `editor.rs` 里“想定时刷新任务列表但返回的是带生命周期的任务对象”这两个问题。 

---

## `types.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraeEditorMode {
    SOLO,
    IDE,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraeEditorPrebuiltSoloAgent {
    Coder,
    Builder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraeTaskStatus {
    Idle,
    Running,
    Interrupted,
    WaitingForHITL,
    Finished,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraeTask {
    pub title: String,
    pub status: TraeTaskStatus,
    pub selected: bool,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraeSoloTaskFeedback {
    Good,
    Bad,
}
```

---

## `task.rs`

```rust
use crate::consts::DEFAULT_SELECTOR_TIMEOUT;
use crate::trae::editor::TraeEditor;
use crate::trae::types::TraeEditorMode;
use crate::utils::wait_for_selector;
use anyhow::{Error, Result};
use chromiumoxide::cdp::browser_protocol::input::InsertTextParams;
use tokio::time::{sleep, Duration};

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

    async fn ensure_solo_mode(&self) -> Result<(), Error> {
        let mode = self.editor.current_mode().await;
        if mode != TraeEditorMode::SOLO {
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

        create_task_button.click().await?;

        self.wait_until_task_creation_page_ready().await?;

        let chat_input_element = wait_for_selector(
            &self.editor.main_page,
            "#agent-chat-view div.chat-input-wrapper div.chat-input-v2-input-box-editable",
            Duration::from_millis(1000 * 60),
        )
        .await?;

        chat_input_element.click().await?;

        self.editor
            .main_page
            .execute(InsertTextParams::new(self.prompt.as_str()))
            .await?;

        sleep(Duration::from_millis(300)).await;

        chat_input_element.press_key("Enter").await?;

        sleep(Duration::from_millis(1000)).await;

        let _ = self.editor.refresh_tasks().await;

        Ok(())
    }
}
```

---

## `editor.rs`

```rust
use crate::config::Config;
use crate::consts::*;
use crate::trae::task::NewTraeTask;
use crate::trae::types::*;
use crate::utils::{normalize_executable_path_for_cdp, wait_for_selector};
use anyhow::{Error, Result};
use chromiumoxide::{cdp::browser_protocol::target::TargetInfo, Browser, Page};
use tokio::{
    sync::{watch, RwLock},
    time::{self, sleep, Duration, MissedTickBehavior},
};

#[derive(Debug)]
pub struct TraeEditor {
    pub(crate) main_page: Page,
    pub(crate) target: TargetInfo,
    pub(crate) prebuilt_agent: TraeEditorPrebuiltSoloAgent,
    pub(crate) mode: RwLock<TraeEditorMode>,
    pub(crate) tasks: RwLock<Vec<TraeTask>>,
}

fn parse_task_status(raw: &str) -> TraeTaskStatus {
    let text = raw.trim();

    match text {
        TRAE_SOLO_TASK_INTERRUPTED_LABEL => TraeTaskStatus::Interrupted,
        TRAE_SOLO_TASK_RUNNING_LABEL => TraeTaskStatus::Running,
        _ => {
            let lower = text.to_ascii_lowercase();

            if lower.contains("hitl") || lower.contains("human in the loop") || text.contains("等待")
            {
                TraeTaskStatus::WaitingForHITL
            } else if lower.contains("finish")
                || lower.contains("done")
                || text.contains("完成")
            {
                TraeTaskStatus::Finished
            } else if lower.contains("idle") || text.contains("空闲") {
                TraeTaskStatus::Idle
            } else {
                TraeTaskStatus::Unknown
            }
        }
    }
}

pub async fn get_current_editor_mode(page: &Page) -> Result<TraeEditorMode, Error> {
    let trae_mode_badge_element = wait_for_selector(
        page,
        "div.fixed-titlebar-container div.icube-mode-tab > div.icube-tooltip-container > div.icube-tooltip-text.icube-simple-style",
        Duration::from_millis(DEFAULT_SELECTOR_TIMEOUT),
    )
    .await?;

    let mode_description = trae_mode_badge_element
        .inner_html()
        .await?
        .unwrap_or_default();

    // 注意：
    // 这里按“badge 显示的是当前模式”来判断。
    // 如果你实际测试发现 badge 显示的是“可切换到的模式”，
    // 那就把下面两个分支对调回来。
    if mode_description.eq(TRAE_SOLO_MODE_TEXT_LABEL) {
        Ok(TraeEditorMode::SOLO)
    } else if mode_description.eq(TRAE_IDE_MODE_TEXT_LABEL) {
        Ok(TraeEditorMode::IDE)
    } else {
        Err(Error::msg(format!(
            "Cannot get the current editor mode, description: {}",
            mode_description
        )))
    }
}

pub struct TraeEditorBuilder;

impl TraeEditorBuilder {
    pub async fn build(&self, browser: &mut Browser) -> TraeEditor {
        let targets = browser.fetch_targets().await.expect("Fetch targets error.");

        sleep(Duration::from_millis(2000)).await;

        let config = Config::load()
            .expect("Cannot load config from TraeEditorBuilder::build, make sure you write config.jsonc properly.");

        let normalized_path =
            normalize_executable_path_for_cdp(&config.trae_executable_path).unwrap();

        let mut filtered_target: Vec<TargetInfo> = targets
            .into_iter()
            .filter(|t| {
                t.url.contains(&format!(
                    "vscode-file://vscode-app/{}/resources/app/out/vs/code/electron-browser/workbench/workbench.html",
                    normalized_path
                ))
            })
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

        let current_mode = get_current_editor_mode(&main_page)
            .await
            .expect("Cannot get current mode when initializing.");

        TraeEditor {
            target: main_target,
            main_page,
            prebuilt_agent: TraeEditorPrebuiltSoloAgent::Coder,
            mode: RwLock::new(current_mode),
            tasks: RwLock::new(Vec::new()),
        }
    }
}

impl TraeEditor {
    pub fn new() -> TraeEditorBuilder {
        TraeEditorBuilder {}
    }

    pub fn get_main_page(&self) -> &Page {
        &self.main_page
    }

    pub async fn current_mode(&self) -> TraeEditorMode {
        *self.mode.read().await
    }

    pub async fn switch_editor_mode(&self, mode: TraeEditorMode) -> Result<(), Error> {
        let current_mode = self.current_mode().await;

        if current_mode == mode {
            return Ok(());
        }

        let trae_mode_tab_switch = self
            .main_page
            .find_element("div.fixed-titlebar-container div.icube-mode-tab > div.icube-mode-tab-container > div.icube-mode-tab-switch")
            .await?;

        trae_mode_tab_switch.click().await?;

        sleep(Duration::from_millis(500)).await;

        let actual_mode = get_current_editor_mode(&self.main_page)
            .await
            .unwrap_or(mode);

        let mut guard = self.mode.write().await;
        *guard = actual_mode;

        Ok(())
    }

    pub fn create_new_task(&self, prompt: impl Into<String>) -> NewTraeTask<'_> {
        NewTraeTask::new(self, prompt.into())
    }

    pub fn set_default_prebuilt_solo_agent(&mut self, agent: TraeEditorPrebuiltSoloAgent) {
        self.prebuilt_agent = agent;
    }

    pub fn get_default_prebuilt_solo_agent(&self) -> TraeEditorPrebuiltSoloAgent {
        self.prebuilt_agent
    }

    pub async fn fetch_tasks_from_ui(&self) -> Result<Vec<TraeTask>, Error> {
        if self.current_mode().await != TraeEditorMode::SOLO {
            return Err(Error::msg(
                "Cannot get tasks under IDE mode, please switch to SOLO mode.",
            ));
        }

        let task_container = self
            .main_page
            .find_element(r#"#solo-ai-sidebar-content div[class*="task-items-list"]"#)
            .await?;

        let task_items = task_container
            .find_elements(r#"div[class*="index-module__task-item___"]"#)
            .await?;

        let mut tasks = Vec::with_capacity(task_items.len());

        for item in task_items {
            let class_name = item.attribute("class").await?.unwrap_or_default();
            let selected = class_name.contains("selected");

            let raw_task_state = match item.find_element(r#"div[class*="task-type-wrap"]"#).await {
                Ok(el) => el.inner_html().await?.unwrap_or_default(),
                Err(_) => String::new(),
            };

            let task_title = match item.find_element(r#"span[class*="task-title"]"#).await {
                Ok(el) => el.inner_html().await?.unwrap_or_default(),
                Err(_) => String::new(),
            };

            if task_title.trim().is_empty() {
                continue;
            }

            let task = TraeTask {
                title: task_title,
                status: parse_task_status(&raw_task_state),
                selected,
            };

            tasks.push(task);
        }

        Ok(tasks)
    }

    pub async fn refresh_tasks(&self) -> Result<Vec<TraeTask>, Error> {
        let latest = self.fetch_tasks_from_ui().await?;
        let mut guard = self.tasks.write().await;
        *guard = latest.clone();
        Ok(latest)
    }

    pub async fn get_tasks(&self) -> Result<Vec<TraeTask>, Error> {
        self.refresh_tasks().await
    }

    pub async fn cached_tasks(&self) -> Vec<TraeTask> {
        self.tasks.read().await.clone()
    }

    pub async fn find_cached_task_by_title(&self, title: &str) -> Option<TraeTask> {
        self.tasks
            .read()
            .await
            .iter()
            .find(|t| t.title == title)
            .cloned()
    }

    pub async fn run_task_sync_loop(
        &self,
        interval: Duration,
        mut shutdown_rx: watch::Receiver<bool>,
    ) {
        let _ = self.refresh_tasks().await;

        let mut ticker = time::interval(interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

        // 丢掉 interval 的第一次立即触发
        ticker.tick().await;

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if let Err(err) = self.refresh_tasks().await {
                        eprintln!("refresh_tasks failed: {err:?}");
                    }
                }
                changed = shutdown_rx.changed() => {
                    match changed {
                        Ok(_) => {
                            if *shutdown_rx.borrow() {
                                break;
                            }
                        }
                        Err(_) => {
                            break;
                        }
                    }
                }
            }
        }
    }
}
```

---

## 外层怎么用这个 loop

这里就是“editor 提供 loop 方法，但不强绑 spawn 策略”的用法。

### 方案 1：当前任务里直接跑

```rust
use tokio::sync::watch;
use tokio::time::Duration;

let (shutdown_tx, shutdown_rx) = watch::channel(false);

editor.run_task_sync_loop(Duration::from_secs(2), shutdown_rx).await;

// 需要停的时候：
// let _ = shutdown_tx.send(true);
```

### 方案 2：外层自己决定是否 `spawn`

如果你的 `TraeEditor` 所在上下文能安全这么做，就由外层去 `spawn`：

```rust
use std::sync::Arc;
use tokio::sync::watch;
use tokio::time::Duration;

let editor = Arc::new(editor);
let editor_for_loop = Arc::clone(&editor);

let (shutdown_tx, shutdown_rx) = watch::channel(false);

tokio::spawn(async move {
    editor_for_loop
        .run_task_sync_loop(Duration::from_secs(2), shutdown_rx)
        .await;
});

// 之后随时读取缓存
let tasks = editor.cached_tasks().await;

// 停止 loop
let _ = shutdown_tx.send(true);
```

如果后面你发现 `Page` / `Browser` 相关对象不是 `Send`，那就不要 `tokio::spawn`，改成：

* 当前 async 任务里直接 `await`
* 或者 `LocalSet` / `spawn_local`
* 或者放进你的 TUI 主循环里手动调 `refresh_tasks()`

这就是“**editor 只提供能力，不替外层决定调度方式**”。

---

## 这版和你原来相比，最核心的变化

原来是：

* `TraeSoloTask<'a>` 持有 `&'a TraeEditor`
* `get_tasks()` 返回 `Vec<TraeSoloTask<'_>>`
* 任务既是数据，又是行为，又是状态机

现在是：

* `TraeTask` 只是纯快照
* `TraeEditor` 内部持有 `RwLock<Vec<TraeTask>>`
* `refresh_tasks()` 负责抓 UI 并更新缓存
* `run_task_sync_loop()` 负责持续同步
* `NewTraeTask` 只负责“创建任务”这一个动作

这样最适合你后面接：

* TUI 展示
* 定时刷新
* 状态订阅
* 更轻量的扩展

你要的话，我下一条可以继续把 `main.rs` 也一起改成这套新 API 的完整可运行版本。
