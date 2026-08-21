# ternary-scheduling: Priority scheduling with ternary decision signals

Priority queues, deadline-aware scheduling, and round-robin dispatch where each task carries a ternary signal: prioritize (+1), defer (−1), or neutral (0).

## Why This Exists

Not all tasks are equal, but binary priority (high/low) is too coarse. Some tasks you want to expedite, some you want to delay, and some are normal. This crate models that as a ternary decision attached to each task, then uses it to influence ordering in priority queues, deadline scheduling, and weighted round-robin dispatch.

## Core Concepts

- **TernaryDecision** — A scheduling signal: `Prioritize` (+1), `Defer` (−1), or `Neutral` (0). Attached to tasks to influence execution order.
- **Task** — A unit of work with an id, name, base priority, ternary signal, optional deadline (epoch ms), and effort (estimated work units).
- **Effective priority** — `base_priority + ternary_signal.value()`. A task with priority 5 and `Defer` signal has effective priority 4; with `Prioritize`, it's 6.
- **Urgency** — A computed score combining effective priority and deadline proximity. Higher urgency = should run sooner. Overdue tasks get maximum urgency.
- **TernaryPriorityQueue** — A max-heap ordered by effective priority. Signal overrides (`set_signal`) **re-heapify**, so promoting a task genuinely moves it up the queue.
- **DeadlineScheduler** — Tracks tasks with deadlines. Can report overdue tasks, schedule by urgency, or schedule by earliest deadline first (EDF).
- **RoundRobinScheduler** — Three queues (prioritize, neutral, defer) with weighted round-robin: prioritize gets 3 slots per cycle, neutral gets 2, defer gets 1.

## Quick Start

```toml
# Cargo.toml
[dependencies]
ternary-scheduling = "0.1"
```

```rust
use ternary_scheduling::*;

// Priority queue with ternary signals
let mut pq = TernaryPriorityQueue::new();
pq.push(Task::new(1, "urgent_report").with_priority(5).with_signal(TernaryDecision::Prioritize));
pq.push(Task::new(2, "daily_sync").with_priority(3).with_signal(TernaryDecision::Neutral));
pq.push(Task::new(3, "backlog_cleanup").with_priority(1).with_signal(TernaryDecision::Defer));

let tasks = pq.drain_sorted();
assert_eq!(tasks[0].name, "urgent_report");   // effective priority: 6
assert_eq!(tasks[1].name, "daily_sync");       // effective priority: 3
assert_eq!(tasks[2].name, "backlog_cleanup");  // effective priority: 0

// Deadline-aware scheduling
let mut ds = DeadlineScheduler::new(0);
ds.add(Task::new(1, "task_a").with_deadline(100).with_effort(10));
ds.add(Task::new(2, "task_b").with_deadline(20).with_effort(5));
let order = ds.schedule_by_deadline();
assert_eq!(order[0].name, "task_b"); // earlier deadline first

// Round-robin with weighted dispatch
let mut rr = RoundRobinScheduler::new();
rr.add(Task::new(1, "p1").with_signal(TernaryDecision::Prioritize));
rr.add(Task::new(2, "d1").with_signal(TernaryDecision::Defer));
assert_eq!(rr.next().unwrap().name, "p1"); // prioritize comes first
```

## API Overview

| Type / Function | What it is |
|---|---|
| `TernaryDecision` | Enum: `Prioritize`, `Defer`, `Neutral` |
| `Task` | Unit of work: id, name, priority, signal, deadline, effort |
| `TernaryPriorityQueue` | Max-heap with signal overrides |
| `DeadlineScheduler` | Time-aware scheduler: overdue detection, urgency/deadline ordering |
| `RoundRobinScheduler` | Weighted round-robin across three signal queues |
| `schedule_min_weighted_completion` | Smith's rule (WSPT): order by effective_priority/effort to minimise total weighted completion time |
| `earliest_deadline_first` | EDF scheduling; returns `None` if infeasible |

## How It Works

**Priority queue.** `Task` implements `Ord` based on effective priority (higher first), with task id as a deterministic tiebreaker (lower id first). `TernaryPriorityQueue` wraps `BinaryHeap<Task>`. `set_signal` drains, mutates the matching task in place, and **re-heapifies**, so an override genuinely changes pop order (this is `O(n)` — set signals before heavy popping). `effective_priority` saturates at the `i32` bounds so a maxed-out priority plus a `Prioritize` signal never overflows.

**Deadline scheduler.** `urgency(current_time)` computes: `effective_priority * 100 − deadline_penalty`, where the penalty is `0` for overdue tasks (`deadline ≤ current_time`, most urgent), `min(deadline_remaining, 10000)` for future deadlines (closer = more urgent, clamped), and `10000` for tasks with no deadline (no time pressure — least urgent). `schedule_by_urgency` sorts descending; `schedule_by_deadline` sorts ascending by deadline, breaking ties by effective priority.

**Round-robin.** Tasks are placed into three internal vectors based on their ternary signal. The dispatch cycle is `[0, 0, 0, 1, 1, 2]` (3 prioritize slots, 2 neutral, 1 defer). `next()` cycles through this pattern, pulling from the first non-empty queue that matches.

**Weighted completion (Smith's rule).** `schedule_min_weighted_completion` orders tasks to minimise the total weighted completion time `Σ weight_j · completion_time_j`. The weight is a task's effective priority and the processing time is its effort. The ordering is by descending `weight / effort`, compared with exact integer cross-multiplication (`w_a·p_b` vs `w_b·p_a`) — no floating point, so ties never suffer rounding jitter. (With all efforts equal this reduces to a plain priority sort.)

**EDF.** `earliest_deadline_first` sorts tasks by deadline and simulates execution, tracking cumulative effort. If any task's completion time would exceed its deadline, the schedule is infeasible and `None` is returned. Cumulative effort that would overflow `u64` is also treated as infeasible (never a panic).

## Known Limitations

- **Round-robin uses `Vec::remove(0)`.** This is O(n) per dispatch. For high-throughput scheduling with thousands of tasks in a single queue, this becomes a bottleneck. Use `VecDeque` if you need better performance.
- **Urgency formula is ad-hoc.** The magic numbers (×100, cap at 10000) work for moderate priority ranges but may produce unexpected orderings when priorities span a wide range (e.g., 0 to 10000).
- **EDF doesn't support preemption.** It assumes tasks run to completion in order. No support for interrupting a running task to handle a higher-urgency one.

## Use Cases

- **Build systems.** Mark compilation targets as prioritize (changed files), defer (generated outputs), or neutral (unchanged). The scheduler runs the most impactful work first.
- **Alert triage.** Incoming alerts carry a ternary urgency signal. Prioritize signals go to the top of the queue; defer signals go to batch processing.
- **Game AI action scheduling.** Actions are tagged prioritize (combat), neutral (patrol), or defer (idle). Round-robin ensures idle actions eventually run even during combat-heavy periods.

## Ecosystem Context

Consumes `TernaryDecision` signals that may come from `ternary-scoring` (scored candidates become scheduling decisions) or `ternary-replay` (replayed decisions drive re-scheduling). No direct dependency on other ternary crates.

## License

MIT

## See Also
- **ternary-sync** — related
- **ternary-room** — related
- **ternary-consensus** — related
- **ternary-bus** — related
- **ternary-event** — related

