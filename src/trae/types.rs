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

// 状态结构体
pub struct Interrupted;
pub struct Running;
pub struct WaitingForHITL;
pub struct Finished;
pub struct Idle;

pub trait TaskState {}
impl TaskState for Interrupted {}
impl TaskState for Running {}
impl TaskState for WaitingForHITL {}
impl TaskState for Finished {}
impl TaskState for Idle {}

pub trait Action {}
impl Action for Interrupted {}
impl Action for Finished {}

pub enum TraeSoloTaskFeedback {
    Good,
    Bad,
}
