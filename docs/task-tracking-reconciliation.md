# Task Tracking Reconciliation

## Why this exists

Trae's sidebar does not expose a stable task identifier in the UI snapshot we currently read.
The old implementation therefore compared tasks by:

- `title`
- `index`

That approach breaks when:

- multiple tasks share the same title
- a task moves to index `0`
- a new task is inserted at the front and pushes every other task down

The result was that one logical task could "steal" another task's slot during diffing, which
caused missed `WaitingForHITL` events and stale follow-up actions.

## New model

We now separate **identity** from **position**:

- `task_id`: stable in-memory identity assigned by this process
- `index`: current sidebar position in the latest UI snapshot

Important distinction:

- `task_id` is how we decide whether two snapshots describe the same logical task
- `index` is only how we physically click the task in the sidebar right now

## Reconciliation flow

Every poll now follows this sequence:

1. Read raw sidebar rows from the DOM.
2. Reconcile the new rows against the previously cached tasks.
3. Preserve the old `task_id` for rows that still look like the same logical task.
4. Assign a fresh `task_id` only to rows that look genuinely new.
5. Diff task status by `task_id`.
6. Execute UI actions by resolving the latest `index` from `task_id` just before clicking.

## Reconciliation assumptions

The implementation is based on the observed Trae behavior:

- a newly created task appears at index `0`
- after sending input to a `Finished` or `Interrupted` task, that task jumps to index `0`
- after interacting with a `WaitingForHITL` task, that task does not move
- when one task jumps to the front, the remaining tasks keep their relative order

These rules are now applied directly instead of being approximated with heuristic scoring.

## Reconciliation strategy

The implementation keeps a tiny explicit hint for the next refresh:

- `NewTaskAtFront`
- `MoveTaskToFront { task_id }`

Those hints are recorded when our own code performs the action that is known to reorder the list:

- creating a new task records `NewTaskAtFront`
- sending follow-up input to a `Finished` or `Interrupted` task records `MoveTaskToFront`

Then reconciliation becomes deterministic:

- if the hint says a new task was created, index `0` gets a fresh `task_id`
- if the hint says a specific task moved to the front, index `0` keeps that task's `task_id`
- otherwise the list is treated as stable, except that length growth is interpreted as new tasks inserted at the front

No title/status scoring is needed in the main path anymore.

## Why actions resolve index late

Even after diffing is fixed, a task may still move between:

- event creation
- event handling
- the exact click that opens the task panel

Because of that, task-scoped actions never trust a previously captured `index`.
Instead, they:

1. keep the stable `task_id`
2. fetch the latest task list
3. resolve the current `index` for that `task_id`
4. click using that fresh index

This avoids acting on the wrong task after a reorder.

## Worked examples

### Example 1: duplicate titles remain distinct

Previous snapshot:

| index | task_id | title         | status          |
|------:|--------:|---------------|-----------------|
| 0     | 1       | Build website | Running         |
| 1     | 2       | Build website | WaitingForHITL  |

Latest snapshot:

| index | title         | status         |
|------:|---------------|----------------|
| 0     | Build website | Running        |
| 1     | Build website | WaitingForHITL |

Expected reconciliation:

- latest row `0` keeps `task_id = 1`
- latest row `1` keeps `task_id = 2`

This is the core fix for duplicate titles.

### Example 2: a new task is inserted at the front

Previous snapshot:

| index | task_id | title            |
|------:|--------:|------------------|
| 0     | 11      | Existing task A  |
| 1     | 12      | Existing task B  |

Latest snapshot:

| index | title            |
|------:|------------------|
| 0     | New task         |
| 1     | Existing task A  |
| 2     | Existing task B  |

Expected reconciliation:

- latest row `0` gets a new `task_id`
- latest row `1` keeps `task_id = 11`
- latest row `2` keeps `task_id = 12`

### Example 3: a terminal task jumps to the front

Previous snapshot:

| index | task_id | title         | status   |
|------:|--------:|---------------|----------|
| 0     | 21      | Build website | Finished |
| 1     | 22      | Build website | Finished |
| 2     | 23      | Other task    | Running  |

After the user sends follow-up input to the second finished task, the latest snapshot becomes:

| index | title         | status   |
|------:|---------------|----------|
| 0     | Build website | Running  |
| 1     | Build website | Finished |
| 2     | Other task    | Running  |

Expected reconciliation:

- latest row `0` keeps `task_id = 22`
- latest row `1` keeps `task_id = 21`
- latest row `2` keeps `task_id = 23`

This is why the implementation records an explicit `MoveTaskToFront { task_id }` hint.

## Diff behavior

Status diffing now uses `task_id` as the primary key.

That means:

- two tasks with the same title can produce two separate events
- a task that changes status after moving to a different index still emits the correct event
- a brand-new `WaitingForHITL` or terminal task can emit an initial event with `previous_status = None`

## Current limitations

- `task_id` is only stable for the current process lifetime
- if the user performs a reorder-causing action directly inside Trae instead of through this automation layer,
  there may be no explicit hint available for that refresh
- in the no-hint case, the implementation intentionally prefers simple structural rules over guesswork

If durable identity across process restarts becomes necessary later, the next step should be
either:

- discovering a native Trae task id from the frontend/runtime state
- or persisting a local identity ledger plus stronger fingerprint data
