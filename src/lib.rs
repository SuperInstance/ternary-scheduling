//! Task scheduling driven by ternary decision signals.
//!
//! Each task carries a [`TernaryDecision`] — prioritize (`+1`), defer (`-1`),
//! or neutral (`0`) — that nudges its position relative to its base priority.
//! The crate builds three schedulers on top of that idea:
//!
//! - a max-heap [`TernaryPriorityQueue`] ordered by *effective* priority,
//! - a time-aware [`DeadlineScheduler`] that ranks tasks by urgency or deadline, and
//! - a [`RoundRobinScheduler`] that doles out time across three queues with a
//!   weighted cadence (3 prioritize : 2 neutral : 1 defer).
//!
//! Two free functions cover classical scheduling objectives:
//! [`schedule_min_weighted_completion`] (Smith's rule) and
//! [`earliest_deadline_first`] (EDF with feasibility checking).
//!
//! # When to use this
//!
//! Reach for `ternary-scheduling` when plain high/low priority is too coarse
//! and you want to express "expedite this", "park that", and "treat this
//! normally" as a first-class signal that influences heap order, deadline
//! ranking, and round-robin fairness. It is dependency-free (`std` only) and
//! small enough to drop into a build system, an alert-triage queue, or a
//! game-AI action loop.
//!
//! # Example
//!
//! ```
//! use ternary_scheduling::{DeadlineScheduler, RoundRobinScheduler, Task, TernaryDecision, TernaryPriorityQueue};
//!
//! let mut pq = TernaryPriorityQueue::new();
//! pq.push(Task::new(1, "urgent").with_priority(5).with_signal(TernaryDecision::Prioritize));
//! pq.push(Task::new(2, "later").with_priority(1).with_signal(TernaryDecision::Defer));
//! assert_eq!(pq.drain_sorted()[0].name, "urgent"); // effective priority 6 beats 0
//!
//! let mut ds = DeadlineScheduler::new(0);
//! ds.add(Task::new(1, "a").with_deadline(100));
//! ds.add(Task::new(2, "b").with_deadline(20));
//! assert_eq!(ds.schedule_by_deadline()[0].name, "b"); // earliest deadline first
//! # let _ = RoundRobinScheduler::new();
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// A three-valued scheduling signal attached to a [`Task`].
///
/// The signal shifts a task's [`Task::effective_priority`] by a small, fixed
/// amount so that "expedite / park / normal" can be expressed without a full
/// priority rewrite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TernaryDecision {
    /// Bump the task up: adds `+1` to effective priority.
    Prioritize,
    /// Push the task down: subtracts `1` from effective priority.
    Defer,
    /// Leave the task where it is: contributes `0`.
    Neutral,
}

impl TernaryDecision {
    /// Numeric weight of the signal: `Prioritize` → `1`, `Defer` → `-1`,
    /// `Neutral` → `0`.
    pub fn value(&self) -> i8 {
        match self {
            TernaryDecision::Prioritize => 1,
            TernaryDecision::Defer => -1,
            TernaryDecision::Neutral => 0,
        }
    }

    /// Decode a numeric weight back into a signal, returning `None` for any
    /// value other than `-1`, `0`, or `1`.
    pub fn from_value(v: i8) -> Option<Self> {
        match v {
            1 => Some(TernaryDecision::Prioritize),
            -1 => Some(TernaryDecision::Defer),
            0 => Some(TernaryDecision::Neutral),
            _ => None,
        }
    }
}

/// A unit of work with a priority, an optional deadline, and a ternary signal.
///
/// Construct one with [`Task::new`] and the `with_*` builder methods. A task's
/// position in every scheduler in this crate is derived from its
/// [`effective_priority`](Task::effective_priority), which folds the ternary
/// signal into the base priority.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Task {
    /// Caller-supplied identifier. Used as a deterministic tiebreaker when two
    /// tasks have equal effective priority (lower id wins).
    pub id: usize,
    /// Human-readable label; not used for ordering.
    pub name: String,
    /// Base priority. Higher means more important. May be negative.
    pub base_priority: i32,
    /// Ternary signal that nudges effective priority by `+1` / `0` / `-1`.
    pub ternary_signal: TernaryDecision,
    /// Absolute deadline (same units as the "current time" passed to a
    /// scheduler, e.g. epoch milliseconds). `None` means "no time pressure".
    pub deadline: Option<u64>,
    /// Estimated work units (processing time). Defaults to `1`. Used by the
    /// weighted-completion and EDF schedulers.
    pub effort: u32,
}

impl Task {
    /// Create a task with the given id and name.
    ///
    /// Defaults: `base_priority = 0`, `ternary_signal = Neutral`, no deadline,
    /// `effort = 1`.
    pub fn new(id: usize, name: impl Into<String>) -> Self {
        Task {
            id,
            name: name.into(),
            base_priority: 0,
            ternary_signal: TernaryDecision::Neutral,
            deadline: None,
            effort: 1,
        }
    }

    /// Builder: set the base priority.
    pub fn with_priority(mut self, p: i32) -> Self {
        self.base_priority = p;
        self
    }

    /// Builder: set the ternary signal.
    pub fn with_signal(mut self, s: TernaryDecision) -> Self {
        self.ternary_signal = s;
        self
    }

    /// Builder: set an absolute deadline.
    pub fn with_deadline(mut self, d: u64) -> Self {
        self.deadline = Some(d);
        self
    }

    /// Builder: set the effort (processing time).
    pub fn with_effort(mut self, e: u32) -> Self {
        self.effort = e;
        self
    }

    /// Effective priority: `base_priority + ternary_signal.value()`, saturated
    /// at the `i32` bounds so it never overflows.
    pub fn effective_priority(&self) -> i32 {
        self.base_priority
            .saturating_add(self.ternary_signal.value() as i32)
    }

    /// Urgency score at `current_time`. Higher means "should run sooner".
    ///
    /// Computed as `effective_priority * 100 - deadline_penalty`, where the
    /// penalty rewards proximity to the deadline:
    ///
    /// - **overdue** (`deadline <= current_time`): penalty `0` (most urgent),
    /// - **future deadline**: penalty `min(deadline - current_time, 10_000)`
    ///   (closer = more urgent, clamped so a far-off deadline can't dominate
    ///   the priority term),
    /// - **no deadline**: penalty `10_000` (no time pressure — least urgent).
    pub fn urgency(&self, current_time: u64) -> i64 {
        let priority_score = self.effective_priority() as i64 * 100;
        let deadline_penalty = match self.deadline {
            Some(dl) if dl <= current_time => 0,
            Some(dl) => (dl - current_time).min(10_000) as i64,
            None => 10_000,
        };
        priority_score - deadline_penalty
    }
}

impl Ord for Task {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher effective priority first (BinaryHeap is a max-heap).
        self.effective_priority()
            .cmp(&other.effective_priority())
            // Tiebreak: lower id first. In a max-heap the "greater" element is
            // popped first, so make the lower id compare as greater.
            .then_with(|| other.id.cmp(&self.id))
    }
}

impl PartialOrd for Task {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A max-heap of [`Task`]s ordered by effective priority.
///
/// Push tasks with [`push`](TernaryPriorityQueue::push) and retrieve them
/// highest-priority-first with [`pop`](TernaryPriorityQueue::pop). A signal
/// [`override`](TernaryPriorityQueue::set_signal) can be applied to a task by id
/// after it has been pushed; the override *re-orders* the heap so it actually
/// changes which task comes out next.
#[derive(Debug, Clone)]
pub struct TernaryPriorityQueue {
    heap: BinaryHeap<Task>,
}

impl TernaryPriorityQueue {
    /// Create an empty queue.
    pub fn new() -> Self {
        TernaryPriorityQueue {
            heap: BinaryHeap::new(),
        }
    }

    /// Push a task into the queue.
    pub fn push(&mut self, task: Task) {
        self.heap.push(task);
    }

    /// Apply a ternary signal to the task with the given id, **re-heapifying**
    /// so the new signal genuinely affects pop order.
    ///
    /// If no task with `task_id` is present this is a no-op. The re-heapify is
    /// `O(n)`, so prefer setting signals before heavy popping.
    pub fn set_signal(&mut self, task_id: usize, signal: TernaryDecision) {
        // Drain, mutate the matching task in place, and rebuild the heap so the
        // override is reflected in ordering rather than only in the popped value.
        let drained = std::mem::take(&mut self.heap);
        for mut task in drained {
            if task.id == task_id {
                task.ternary_signal = signal;
            }
            self.heap.push(task);
        }
    }

    /// Pop the highest-effective-priority task, or `None` if empty.
    pub fn pop(&mut self) -> Option<Task> {
        self.heap.pop()
    }

    /// Peek at the highest-effective-priority task without removing it.
    pub fn peek(&self) -> Option<&Task> {
        self.heap.peek()
    }

    /// Number of tasks currently in the queue.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Whether the queue holds no tasks.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Remove and return every task in priority (highest-first) order.
    pub fn drain_sorted(&mut self) -> Vec<Task> {
        let mut tasks = Vec::with_capacity(self.heap.len());
        while let Some(task) = self.pop() {
            tasks.push(task);
        }
        tasks
    }
}

impl Default for TernaryPriorityQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// A time-aware scheduler that ranks tasks by urgency or deadline.
///
/// Create one with [`DeadlineScheduler::new`] at a starting time, [`add`](
/// DeadlineScheduler::add) tasks, then query [`overdue`](
/// DeadlineScheduler::overdue), [`schedule_by_urgency`](
/// DeadlineScheduler::schedule_by_urgency), or [`schedule_by_deadline`](
/// DeadlineScheduler::schedule_by_deadline). Advance the clock with
/// [`advance_time`](DeadlineScheduler::advance_time).
#[derive(Debug, Clone)]
pub struct DeadlineScheduler {
    tasks: Vec<Task>,
    current_time: u64,
}

impl DeadlineScheduler {
    /// Create an empty scheduler whose clock starts at `current_time`.
    pub fn new(current_time: u64) -> Self {
        DeadlineScheduler {
            tasks: Vec::new(),
            current_time,
        }
    }

    /// Add a task to the scheduler.
    pub fn add(&mut self, task: Task) {
        self.tasks.push(task);
    }

    /// Advance the scheduler's clock by `dt`, saturating at `u64::MAX`.
    pub fn advance_time(&mut self, dt: u64) {
        self.current_time = self.current_time.saturating_add(dt);
    }

    /// Tasks whose deadline has arrived or passed (`deadline <= current_time`).
    pub fn overdue(&self) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| t.deadline.is_some_and(|dl| dl <= self.current_time))
            .collect()
    }

    /// Tasks sorted most-urgent-first (descending [`Task::urgency`]).
    pub fn schedule_by_urgency(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.iter().collect();
        tasks.sort_by(|a, b| {
            b.urgency(self.current_time)
                .cmp(&a.urgency(self.current_time))
        });
        tasks
    }

    /// Tasks sorted earliest-deadline-first. Tasks without a deadline sort last
    /// (as if their deadline were `u64::MAX`); ties are broken by higher
    /// effective priority first.
    pub fn schedule_by_deadline(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.iter().collect();
        tasks.sort_by(|a, b| {
            let da = a.deadline.unwrap_or(u64::MAX);
            let db = b.deadline.unwrap_or(u64::MAX);
            da.cmp(&db)
                .then_with(|| b.effective_priority().cmp(&a.effective_priority()))
        });
        tasks
    }

    /// Remove and return the task with the given id, or `None` if absent.
    pub fn remove(&mut self, id: usize) -> Option<Task> {
        if let Some(pos) = self.tasks.iter().position(|t| t.id == id) {
            Some(self.tasks.remove(pos))
        } else {
            None
        }
    }

    /// Number of tasks tracked by the scheduler.
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Whether the scheduler tracks no tasks.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Current scheduler time.
    pub fn current_time(&self) -> u64 {
        self.current_time
    }

    /// Bulk-apply ternary signals: each `(task_id, decision)` pair sets the
    /// signal on the matching task (tasks are matched by id; unknown ids are
    /// ignored).
    pub fn apply_decisions(&mut self, decisions: &[(usize, TernaryDecision)]) {
        for (task_id, decision) in decisions {
            for task in &mut self.tasks {
                if task.id == *task_id {
                    task.ternary_signal = *decision;
                }
            }
        }
    }
}

/// Round-robin dispatch across three queues (prioritize, neutral, defer).
///
/// Tasks are filed into a queue by their ternary signal. [`next`](
/// RoundRobinScheduler::next) cycles a weighted cadence — three slots for the
/// prioritize queue, two for neutral, one for defer — pulling from the first
/// non-empty queue each step. [`RoundRobinScheduler`] implements [`Iterator`],
/// so you can drive it in a `for` loop or with `.collect()`.
#[derive(Debug, Clone)]
pub struct RoundRobinScheduler {
    queues: [Vec<Task>; 3], // [Prioritize, Neutral, Defer]
    position: usize,
}

impl RoundRobinScheduler {
    /// Create an empty round-robin scheduler.
    pub fn new() -> Self {
        RoundRobinScheduler {
            queues: [Vec::new(), Vec::new(), Vec::new()],
            position: 0,
        }
    }

    /// File a task into the queue matching its ternary signal.
    pub fn add(&mut self, task: Task) {
        let idx = match task.ternary_signal {
            TernaryDecision::Prioritize => 0,
            TernaryDecision::Neutral => 1,
            TernaryDecision::Defer => 2,
        };
        self.queues[idx].push(task);
    }

    /// Total tasks across all three queues.
    pub fn len(&self) -> usize {
        self.queues.iter().map(|q| q.len()).sum()
    }

    /// Whether all three queues are empty.
    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(|q| q.is_empty())
    }

    /// Tasks currently in the prioritize queue.
    pub fn prioritize_count(&self) -> usize {
        self.queues[0].len()
    }

    /// Tasks currently in the defer queue.
    pub fn defer_count(&self) -> usize {
        self.queues[2].len()
    }
}

impl Default for RoundRobinScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Iterator for RoundRobinScheduler {
    type Item = Task;

    /// Pull the next task using the weighted cadence
    /// `[prioritize, prioritize, prioritize, neutral, neutral, defer]`,
    /// skipping empty queues. If a full weighted cycle finds nothing, fall back
    /// to the first non-empty queue. Returns `None` only when every queue is
    /// empty.
    fn next(&mut self) -> Option<Task> {
        let order = [0, 0, 0, 1, 1, 2]; // weighted round-robin
        for _ in 0..order.len() {
            let queue_idx = order[self.position % order.len()];
            self.position = self.position.wrapping_add(1);
            if !self.queues[queue_idx].is_empty() {
                return Some(self.queues[queue_idx].remove(0));
            }
        }
        // Fallback: a full weighted cycle found nothing, so serve whatever
        // queue still has work (keeps deferred tasks draining even when no
        // prioritize/neutral slot hit this cycle).
        for q in &mut self.queues {
            if !q.is_empty() {
                return Some(q.remove(0));
            }
        }
        None
    }
}

/// Order tasks to minimise total weighted completion time.
///
/// Implements **Smith's rule** (a.k.a. weighted-shortest-processing-time):
/// schedule jobs in descending order of `weight / processing_time`, where the
/// weight is the task's [`effective_priority`](Task::effective_priority) and the
/// processing time is its [`effort`](Task::effort). This ordering is optimal for
/// minimising `Σ weight_j · completion_time_j` when weights are non-negative.
///
/// Comparison uses integer cross-multiplication (`w_a·p_b` vs `w_b·p_a`), so it
/// is exact and never touches floating point. Zero `effort` is treated as `1`
/// (a zero-duration task completes instantly regardless of position). The
/// returned `Vec` holds the original slice indices in schedule order.
pub fn schedule_min_weighted_completion(tasks: &[Task]) -> Vec<usize> {
    let mut indexed: Vec<(usize, &Task)> = tasks.iter().enumerate().collect();
    indexed.sort_by(|(_, a), (_, b)| {
        // Order a before b iff a's ratio w/p exceeds b's, i.e.
        // w_a / p_a > w_b / p_b  ⟺  w_a · p_b > w_b · p_a.
        let wa = a.effective_priority() as i64;
        let wb = b.effective_priority() as i64;
        let pa = a.effort.max(1) as i64;
        let pb = b.effort.max(1) as i64;
        // sort_by is ascending; returning rhs.cmp(&lhs) yields descending ratio.
        (wb * pa).cmp(&(wa * pb))
    });
    indexed.iter().map(|(i, _)| *i).collect()
}

/// Earliest-deadline-first scheduling with feasibility checking.
///
/// Tasks are ordered by ascending deadline (tasks without a deadline sort last
/// and impose no constraint). Execution is then simulated: cumulative effort
/// accumulates and, if any task's completion time would exceed its deadline,
/// the instance is infeasible and `None` is returned. Effort totals that would
/// overflow `u64` are also treated as infeasible (never a panic).
///
/// On success the returned `Vec` holds the original slice indices in schedule
/// order.
pub fn earliest_deadline_first(tasks: &[Task]) -> Option<Vec<usize>> {
    let mut indexed: Vec<(usize, &Task)> = tasks.iter().enumerate().collect();
    indexed.sort_by_key(|(_, t)| t.deadline.unwrap_or(u64::MAX));

    let mut time: u64 = 0;
    let mut result = Vec::with_capacity(tasks.len());
    for (orig_idx, task) in &indexed {
        // Cumulative effort overflow is treated as infeasible (never a panic).
        time = time.checked_add(task.effort as u64)?;
        if let Some(dl) = task.deadline {
            if time > dl {
                return None; // completion time exceeds deadline
            }
        }
        result.push(*orig_idx);
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ternary_decision_values() {
        assert_eq!(TernaryDecision::Prioritize.value(), 1);
        assert_eq!(TernaryDecision::Defer.value(), -1);
        assert_eq!(TernaryDecision::Neutral.value(), 0);
    }

    #[test]
    fn test_ternary_from_value() {
        assert_eq!(
            TernaryDecision::from_value(1),
            Some(TernaryDecision::Prioritize)
        );
        assert_eq!(
            TernaryDecision::from_value(-1),
            Some(TernaryDecision::Defer)
        );
        assert_eq!(
            TernaryDecision::from_value(0),
            Some(TernaryDecision::Neutral)
        );
        assert_eq!(TernaryDecision::from_value(5), None);
        assert_eq!(TernaryDecision::from_value(-2), None);
        assert_eq!(TernaryDecision::from_value(i8::MIN), None);
        assert_eq!(TernaryDecision::from_value(i8::MAX), None);
    }

    #[test]
    fn test_task_effective_priority() {
        let t = Task::new(1, "test")
            .with_priority(5)
            .with_signal(TernaryDecision::Prioritize);
        assert_eq!(t.effective_priority(), 6);
    }

    #[test]
    fn test_task_effective_priority_defer() {
        let t = Task::new(1, "test")
            .with_priority(5)
            .with_signal(TernaryDecision::Defer);
        assert_eq!(t.effective_priority(), 4);
    }

    #[test]
    fn test_effective_priority_saturates() {
        // i32::MAX + 1 must saturate rather than panic / wrap.
        let top = Task::new(0, "top")
            .with_priority(i32::MAX)
            .with_signal(TernaryDecision::Prioritize);
        assert_eq!(top.effective_priority(), i32::MAX);

        let bottom = Task::new(0, "bottom")
            .with_priority(i32::MIN)
            .with_signal(TernaryDecision::Defer);
        assert_eq!(bottom.effective_priority(), i32::MIN);
    }

    #[test]
    fn test_priority_queue_ordering() {
        let mut pq = TernaryPriorityQueue::new();
        pq.push(Task::new(1, "low").with_priority(1));
        pq.push(Task::new(2, "high").with_priority(10));
        pq.push(Task::new(3, "mid").with_priority(5));

        let tasks = pq.drain_sorted();
        // Ord: higher effective_priority first; on tie, lower id first.
        assert_eq!(tasks[0].id, 2); // priority 10
        assert_eq!(tasks[1].id, 3); // priority 5
        assert_eq!(tasks[2].id, 1); // priority 1
    }

    #[test]
    fn test_priority_queue_with_signals() {
        let mut pq = TernaryPriorityQueue::new();
        pq.push(
            Task::new(1, "a")
                .with_priority(5)
                .with_signal(TernaryDecision::Defer),
        );
        pq.push(
            Task::new(2, "b")
                .with_priority(3)
                .with_signal(TernaryDecision::Prioritize),
        );

        let tasks = pq.drain_sorted();
        // Task 1: 5-1=4, Task 2: 3+1=4, tied effective priority, lower id first.
        assert_eq!(tasks[0].id, 1);
        assert_eq!(tasks[1].id, 2);
    }

    #[test]
    fn test_priority_queue_tiebreak_is_lower_id_first() {
        // Independent hand-check of the tie-break rule: three equal-priority
        // tasks must emerge in ascending id order from the max-heap.
        let mut pq = TernaryPriorityQueue::new();
        pq.push(Task::new(30, "c").with_priority(7));
        pq.push(Task::new(10, "a").with_priority(7));
        pq.push(Task::new(20, "b").with_priority(7));
        let ids: Vec<usize> = pq.drain_sorted().iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![10, 20, 30]);
    }

    #[test]
    fn test_priority_queue_signal_override_no_change_when_lower() {
        let mut pq = TernaryPriorityQueue::new();
        pq.push(Task::new(1, "a").with_priority(5));
        pq.push(Task::new(2, "b").with_priority(3));
        pq.set_signal(2, TernaryDecision::Prioritize);

        let first = pq.pop().unwrap();
        // After re-heapify: task 2 -> eff 4, task 1 -> eff 5; task 1 still wins.
        assert_eq!(first.id, 1);
    }

    #[test]
    fn test_priority_queue_signal_override_reorders_heap() {
        // SABOTAGE GUARD: two tasks share effective priority 5. Promoting task 2
        // to Prioritize must lift it to eff 6 and change the pop order — the old
        // "override only applied at pop, no reorder" behaviour returned task 1
        // here, so this test fails against that code.
        let mut pq = TernaryPriorityQueue::new();
        pq.push(Task::new(1, "a").with_priority(5));
        pq.push(Task::new(2, "b").with_priority(5));
        pq.set_signal(2, TernaryDecision::Prioritize);

        let first = pq.pop().unwrap();
        assert_eq!(first.id, 2); // override re-ordered the heap
        assert_eq!(first.ternary_signal, TernaryDecision::Prioritize);
        let second = pq.pop().unwrap();
        assert_eq!(second.id, 1);
    }

    #[test]
    fn test_priority_queue_set_signal_unknown_id_is_noop() {
        let mut pq = TernaryPriorityQueue::new();
        pq.push(Task::new(1, "a").with_priority(5));
        pq.set_signal(999, TernaryDecision::Defer); // unknown id
        assert_eq!(pq.len(), 1);
        let t = pq.pop().unwrap();
        assert_eq!(t.ternary_signal, TernaryDecision::Neutral); // unchanged
    }

    #[test]
    fn test_priority_queue_empty() {
        let mut pq = TernaryPriorityQueue::new();
        assert!(pq.is_empty());
        assert!(pq.pop().is_none());
        assert!(pq.peek().is_none());
        assert!(pq.drain_sorted().is_empty());
    }

    #[test]
    fn test_priority_queue_single_task() {
        let mut pq = TernaryPriorityQueue::new();
        pq.push(Task::new(42, "only").with_priority(3));
        assert!(!pq.is_empty());
        assert_eq!(pq.len(), 1);
        assert_eq!(pq.peek().unwrap().id, 42);
        let t = pq.pop().unwrap();
        assert_eq!(t.id, 42);
        assert!(pq.is_empty());
    }

    #[test]
    fn test_deadline_scheduler_overdue() {
        let mut ds = DeadlineScheduler::new(100);
        ds.add(Task::new(1, "late").with_deadline(50));
        ds.add(Task::new(2, "ontime").with_deadline(200));
        ds.add(Task::new(3, "nodeadline"));

        let overdue = ds.overdue();
        assert_eq!(overdue.len(), 1);
        assert_eq!(overdue[0].id, 1);
    }

    #[test]
    fn test_deadline_scheduler_overdue_at_exact_deadline() {
        // dl == current_time counts as overdue (boundary of the <= rule).
        let mut ds = DeadlineScheduler::new(100);
        ds.add(Task::new(1, "now").with_deadline(100));
        assert_eq!(ds.overdue().len(), 1);
    }

    #[test]
    fn test_deadline_scheduler_schedule_by_deadline() {
        let mut ds = DeadlineScheduler::new(0);
        ds.add(Task::new(1, "late").with_deadline(100));
        ds.add(Task::new(2, "early").with_deadline(10));

        let scheduled = ds.schedule_by_deadline();
        assert_eq!(scheduled[0].id, 2);
        assert_eq!(scheduled[1].id, 1);
    }

    #[test]
    fn test_deadline_scheduler_remove() {
        let mut ds = DeadlineScheduler::new(0);
        ds.add(Task::new(1, "a"));
        ds.add(Task::new(2, "b"));
        let removed = ds.remove(1).unwrap();
        assert_eq!(removed.name, "a");
        assert_eq!(ds.len(), 1);
        assert!(ds.remove(999).is_none());
    }

    #[test]
    fn test_deadline_scheduler_apply_decisions() {
        let mut ds = DeadlineScheduler::new(0);
        ds.add(Task::new(1, "a").with_priority(5));
        ds.apply_decisions(&[(1, TernaryDecision::Prioritize)]);
        assert_eq!(ds.tasks[0].ternary_signal, TernaryDecision::Prioritize);
    }

    #[test]
    fn test_deadline_scheduler_advance_time() {
        let mut ds = DeadlineScheduler::new(10);
        ds.advance_time(5);
        assert_eq!(ds.current_time(), 15);
        // Saturates instead of overflowing.
        ds.advance_time(u64::MAX);
        assert_eq!(ds.current_time(), u64::MAX);
    }

    #[test]
    fn test_round_robin_favors_prioritize() {
        let mut rr = RoundRobinScheduler::new();
        rr.add(Task::new(1, "defer").with_signal(TernaryDecision::Defer));
        rr.add(Task::new(2, "prioritize").with_signal(TernaryDecision::Prioritize));

        let first = rr.next().unwrap();
        assert_eq!(first.id, 2);
    }

    #[test]
    fn test_round_robin_all_queues() {
        let mut rr = RoundRobinScheduler::new();
        rr.add(Task::new(1, "p1").with_signal(TernaryDecision::Prioritize));
        rr.add(Task::new(2, "p2").with_signal(TernaryDecision::Prioritize));
        rr.add(Task::new(3, "n1").with_signal(TernaryDecision::Neutral));
        rr.add(Task::new(4, "d1").with_signal(TernaryDecision::Defer));

        // RoundRobinScheduler implements Iterator, so it works with collect().
        let tasks: Vec<Task> = rr.collect();
        let ids: Vec<usize> = tasks.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 4);
        // Prioritize tasks should come first.
        assert!(ids[0] == 1 || ids[0] == 2);
        // Deferred task comes last (drained only via the weighted fallback).
        assert_eq!(ids[3], 4);
    }

    #[test]
    fn test_round_robin_empty() {
        let mut rr = RoundRobinScheduler::new();
        assert!(rr.is_empty());
        assert!(rr.next().is_none());
    }

    #[test]
    fn test_round_robin_counts() {
        let mut rr = RoundRobinScheduler::new();
        rr.add(Task::new(1, "p").with_signal(TernaryDecision::Prioritize));
        rr.add(Task::new(2, "n").with_signal(TernaryDecision::Neutral));
        rr.add(Task::new(3, "d").with_signal(TernaryDecision::Defer));
        assert_eq!(rr.prioritize_count(), 1);
        assert_eq!(rr.defer_count(), 1);
        assert_eq!(rr.len(), 3);
    }

    #[test]
    fn test_min_weighted_completion_simple() {
        let tasks = vec![
            Task::new(0, "low").with_priority(1),
            Task::new(1, "high").with_priority(10),
            Task::new(2, "mid").with_priority(5),
        ];
        let order = schedule_min_weighted_completion(&tasks);
        // All efforts are 1, so ratio == priority → high, mid, low.
        assert_eq!(order, vec![1, 2, 0]);
    }

    #[test]
    fn test_min_weighted_completion_smith_rule_optimal() {
        // Hand-derived: a low-priority/short job beats a high-priority/long job
        // because its weight/time ratio is higher.
        //   Task 0: weight 5, effort 10 -> ratio 0.5, completion if first = 10
        //   Task 1: weight 4, effort  2 -> ratio 2.0, completion if first =  2
        // Smith order: 1 then 0.
        //   ΣwC([1,0]) = 4*2 + 5*12 = 8 + 60 = 68  (optimal)
        //   ΣwC([0,1]) = 5*10 + 4*12 = 50 + 48 = 98 (what the old priority-only
        //                                          sort returned — suboptimal)
        // The old code sorted by priority alone (5 > 4) and produced [0,1].
        let tasks = vec![
            Task::new(0, "heavy").with_priority(5).with_effort(10),
            Task::new(1, "light").with_priority(4).with_effort(2),
        ];
        let order = schedule_min_weighted_completion(&tasks);
        assert_eq!(order, vec![1, 0]);

        // Verify the objective directly: the chosen order must beat the reverse.
        let cost = |perm: &[usize]| -> i64 {
            let mut t = 0i64;
            let mut sum = 0i64;
            for &i in perm {
                t += tasks[i].effort as i64;
                sum += tasks[i].effective_priority() as i64 * t;
            }
            sum
        };
        assert!(cost(&order) < cost(&[0, 1]));
    }

    #[test]
    fn test_min_weighted_completion_empty_and_single() {
        assert!(schedule_min_weighted_completion(&[]).is_empty());
        assert_eq!(
            schedule_min_weighted_completion(&[Task::new(7, "x")]),
            vec![0]
        );
    }

    #[test]
    fn test_min_weighted_completion_ties_stable_by_input() {
        // Equal ratio on distinct (priority, effort) pairs resolves to a stable
        // ordering by original index (no float jitter).
        let tasks = vec![
            Task::new(0, "a").with_priority(2).with_effort(2), // ratio 1
            Task::new(1, "b").with_priority(4).with_effort(4), // ratio 1
            Task::new(2, "c").with_priority(3).with_effort(3), // ratio 1
        ];
        let order = schedule_min_weighted_completion(&tasks);
        assert_eq!(order, vec![0, 1, 2]); // input order preserved on ties
    }

    #[test]
    fn test_earliest_deadline_first() {
        let tasks = vec![
            Task::new(0, "late").with_deadline(100).with_effort(10),
            Task::new(1, "early").with_deadline(20).with_effort(10),
        ];
        let order = earliest_deadline_first(&tasks).unwrap();
        assert_eq!(order[0], 1);
        assert_eq!(order[1], 0);
    }

    #[test]
    fn test_earliest_deadline_infeasible() {
        let tasks = vec![Task::new(0, "long").with_deadline(5).with_effort(10)];
        assert!(earliest_deadline_first(&tasks).is_none());
    }

    #[test]
    fn test_earliest_deadline_no_deadlines() {
        let tasks = vec![Task::new(0, "a"), Task::new(1, "b")];
        assert_eq!(earliest_deadline_first(&tasks).unwrap(), vec![0, 1]);
    }

    #[test]
    fn test_earliest_deadline_empty_and_single() {
        assert_eq!(earliest_deadline_first(&[]), Some(vec![]));
        let tasks = vec![Task::new(9, "solo").with_effort(4)];
        assert_eq!(earliest_deadline_first(&tasks), Some(vec![0]));
    }

    #[test]
    fn test_earliest_deadline_tight_feasible_boundary() {
        // completion == deadline is still feasible (uses strict > for infeasible).
        let tasks = vec![Task::new(0, "exact").with_deadline(10).with_effort(10)];
        assert_eq!(earliest_deadline_first(&tasks), Some(vec![0]));
    }

    #[test]
    fn test_earliest_deadline_deadless_after_deadlined() {
        // Tasks without a deadline sort last and never make a schedule infeasible.
        let tasks = vec![
            Task::new(0, "free").with_effort(1000),
            Task::new(1, "dl").with_deadline(5).with_effort(5),
        ];
        let order = earliest_deadline_first(&tasks).unwrap();
        assert_eq!(order, vec![1, 0]); // deadline task first
    }

    #[test]
    fn test_task_urgency() {
        let t1 = Task::new(1, "urgent").with_priority(10).with_deadline(10);
        let t2 = Task::new(2, "relaxed").with_priority(1).with_deadline(1000);
        let u1 = t1.urgency(5);
        let u2 = t2.urgency(5);
        assert!(u1 > u2);
    }

    #[test]
    fn test_urgency_ranking_matches_documented_formula() {
        // Independent derivation of the documented formula:
        //   urgency = eff_priority*100 - deadline_penalty
        // where penalty is 0 (overdue) / min(remaining, 10000) (future) /
        // 10000 (no deadline).
        let now = 100u64;
        let overdue = Task::new(0, "o").with_priority(3).with_deadline(50); // dl<=now -> 0
        let near = Task::new(1, "n").with_priority(3).with_deadline(150); // remaining 50
        let far = Task::new(2, "f").with_priority(3).with_deadline(100_000); // remaining clamped 10000
        let none = Task::new(3, "x").with_priority(3); // no deadline -> 10000

        // Expected: 3*100 - {0, 50, 10000, 10000} = {300, 250, -9700, -9700}
        assert_eq!(overdue.urgency(now), 300);
        assert_eq!(near.urgency(now), 250);
        assert_eq!(far.urgency(now), -9700);
        assert_eq!(none.urgency(now), -9700);
        assert!(overdue.urgency(now) > near.urgency(now));
    }

    #[test]
    fn test_task_builder() {
        let t = Task::new(42, "complex")
            .with_priority(7)
            .with_signal(TernaryDecision::Prioritize)
            .with_deadline(100)
            .with_effort(5);
        assert_eq!(t.id, 42);
        assert_eq!(t.name, "complex");
        assert_eq!(t.base_priority, 7);
        assert_eq!(t.ternary_signal, TernaryDecision::Prioritize);
        assert_eq!(t.deadline, Some(100));
        assert_eq!(t.effort, 5);
    }
}
