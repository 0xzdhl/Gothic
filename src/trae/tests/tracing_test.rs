use super::super::editor::{
    PendingUiActionRetry, TaskListHint, collect_pending_retry_events, reconcile_task_snapshots,
};
use super::super::types::{TraeTask, TraeTaskStatus};
use super::super::workflow::diff_task_status_changes;
use std::collections::HashMap;

const MAX_TASK_ACTION_RETRY: u32 = 3;

fn task(
    task_id: u64,
    title: &str,
    status: TraeTaskStatus,
    selected: bool,
    index: usize,
) -> TraeTask {
    TraeTask {
        task_id,
        title: title.to_string(),
        status,
        selected,
        index,
    }
}

#[test]
fn reconcile_keeps_stable_order_without_hint() {
    let previous = vec![
        task(1, "Build website", TraeTaskStatus::Running, false, 0),
        task(2, "Build website", TraeTaskStatus::WaitingForHITL, true, 1),
        task(3, "Other task", TraeTaskStatus::Running, false, 2),
    ];

    assert_eq!(
        reconcile_task_snapshots(&previous, previous.len(), None),
        vec![Some(0), Some(1), Some(2)]
    );
}

#[test]
fn reconcile_assigns_a_new_front_task_from_explicit_hint() {
    let previous = vec![
        task(11, "Existing task A", TraeTaskStatus::Running, false, 0),
        task(
            12,
            "Existing task B",
            TraeTaskStatus::WaitingForHITL,
            true,
            1,
        ),
    ];

    assert_eq!(
        reconcile_task_snapshots(&previous, 3, Some(TaskListHint::NewTaskAtFront)),
        vec![None, Some(0), Some(1)]
    );
}

#[test]
fn reconcile_infers_new_front_tasks_from_length_growth() {
    let previous = vec![
        task(11, "Existing task A", TraeTaskStatus::Running, false, 0),
        task(
            12,
            "Existing task B",
            TraeTaskStatus::WaitingForHITL,
            true,
            1,
        ),
    ];

    assert_eq!(
        reconcile_task_snapshots(&previous, 3, None),
        vec![None, Some(0), Some(1)]
    );
}

#[test]
fn reconcile_moves_the_known_terminal_task_to_front() {
    let previous = vec![
        task(21, "Build website", TraeTaskStatus::Finished, false, 0),
        task(22, "Build website", TraeTaskStatus::Finished, true, 1),
        task(23, "Other task", TraeTaskStatus::Running, false, 2),
    ];

    assert_eq!(
        reconcile_task_snapshots(
            &previous,
            3,
            Some(TaskListHint::MoveTaskToFront { task_id: 22 }),
        ),
        vec![Some(1), Some(0), Some(2)]
    );
}

#[test]
fn diff_uses_task_id_for_duplicate_titles() {
    let previous = vec![
        task(1, "Build website", TraeTaskStatus::WaitingForHITL, false, 0),
        task(2, "Build website", TraeTaskStatus::Running, false, 1),
    ];
    let latest = vec![
        task(1, "Build website", TraeTaskStatus::Running, false, 1),
        task(2, "Build website", TraeTaskStatus::WaitingForHITL, false, 0),
    ];

    let events = diff_task_status_changes(&previous, &latest);

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].task_id(), 1);
    assert_eq!(
        events[0].previous_status,
        Some(TraeTaskStatus::WaitingForHITL)
    );
    assert_eq!(events[1].task_id(), 2);
    assert_eq!(events[1].previous_status, Some(TraeTaskStatus::Running));
}

#[test]
fn diff_emits_initial_event_for_new_waiting_task() {
    let previous = vec![task(1, "Existing task", TraeTaskStatus::Running, false, 0)];
    let latest = vec![
        task(2, "New task", TraeTaskStatus::WaitingForHITL, false, 0),
        task(1, "Existing task", TraeTaskStatus::Running, false, 1),
    ];

    let events = diff_task_status_changes(&previous, &latest);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].task_id(), 2);
    assert_eq!(events[0].previous_status, None);
    assert_eq!(events[0].current_status(), TraeTaskStatus::WaitingForHITL);
}

#[test]
fn retry_events_reemit_actionable_tasks_that_stay_stuck() {
    let latest = vec![task(7, "Retry me", TraeTaskStatus::WaitingForHITL, true, 0)];
    let mut pending = HashMap::from([(
        7,
        PendingUiActionRetry {
            status: TraeTaskStatus::WaitingForHITL,
            attempts: 1,
            warned: false,
        },
    )]);

    let (events, warned) =
        collect_pending_retry_events(&latest, &mut pending, MAX_TASK_ACTION_RETRY);

    assert_eq!(warned.len(), 0);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].task_id(), 7);
    assert_eq!(
        events[0].previous_status,
        Some(TraeTaskStatus::WaitingForHITL)
    );
}

#[test]
fn retry_events_drop_entries_after_status_changes() {
    let latest = vec![task(7, "Retry me", TraeTaskStatus::Running, true, 0)];
    let mut pending = HashMap::from([(
        7,
        PendingUiActionRetry {
            status: TraeTaskStatus::WaitingForHITL,
            attempts: 1,
            warned: false,
        },
    )]);

    let (events, warned) =
        collect_pending_retry_events(&latest, &mut pending, MAX_TASK_ACTION_RETRY);

    assert!(events.is_empty());
    assert!(warned.is_empty());
    assert!(pending.is_empty());
}

#[test]
fn retry_events_warn_once_after_reaching_attempt_limit() {
    let latest = vec![task(7, "Retry me", TraeTaskStatus::Interrupted, true, 0)];
    let mut pending = HashMap::from([(
        7,
        PendingUiActionRetry {
            status: TraeTaskStatus::Interrupted,
            attempts: MAX_TASK_ACTION_RETRY,
            warned: false,
        },
    )]);

    let (events, warned) =
        collect_pending_retry_events(&latest, &mut pending, MAX_TASK_ACTION_RETRY);

    assert!(events.is_empty());
    assert_eq!(warned.len(), 1);
    assert_eq!(warned[0].task_id, 7);
    assert_eq!(pending.get(&7).unwrap().warned, true);

    let (events_again, warned_again) =
        collect_pending_retry_events(&latest, &mut pending, MAX_TASK_ACTION_RETRY);
    assert!(events_again.is_empty());
    assert!(warned_again.is_empty());
}
