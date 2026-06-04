//! Task scheduling using ternary decisions.
//!
//! Priority queues with ternary signals, deadline-aware scheduling,
//! and task ordering based on prioritize (+1), defer (-1), or neutral (0).

#![forbid(unsafe_code)]

use std::collections::{BinaryHeap, HashMap};
use std::cmp::Ordering;

/// Ternary decision for task scheduling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TernaryDecision {
    Prioritize,
    Defer,
    Neutral,
}

impl TernaryDecision {
    pub fn value(&self) -> i8 {
        match self {
            TernaryDecision::Prioritize => 1,
            TernaryDecision::Defer => -1,
            TernaryDecision::Neutral => 0,
        }
    }

    pub fn from_value(v: i8) -> Option<Self> {
        match v {
            1 => Some(TernaryDecision::Prioritize),
            -1 => Some(TernaryDecision::Defer),
            0 => Some(TernaryDecision::Neutral),
            _ => None,
        }
    }
}

/// A task with priority, deadline, and ternary signal.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Task {
    pub id: usize,
    pub name: String,
    pub base_priority: i32,
    pub ternary_signal: TernaryDecision,
    pub deadline: Option<u64>,
    pub effort: u32,
}

impl Task {
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

    pub fn with_priority(mut self, p: i32) -> Self {
        self.base_priority = p;
        self
    }

    pub fn with_signal(mut self, s: TernaryDecision) -> Self {
        self.ternary_signal = s;
        self
    }

    pub fn with_deadline(mut self, d: u64) -> Self {
        self.deadline = Some(d);
        self
    }

    pub fn with_effort(mut self, e: u32) -> Self {
        self.effort = e;
        self
    }

    /// Effective priority combines base priority with ternary signal.
    pub fn effective_priority(&self) -> i32 {
        self.base_priority + self.ternary_signal.value() as i32
    }

    /// Urgency score: higher means more urgent.
    /// Factors in effective priority and deadline proximity.
    pub fn urgency(&self, current_time: u64) -> i64 {
        let priority_score = self.effective_priority() as i64 * 100;
        let deadline_score = match self.deadline {
            Some(dl) if dl > current_time => (dl - current_time) as i64,
            Some(_) => 0, // overdue — highest urgency
            None => i64::MAX / 2, // no deadline — medium urgency
        };
        // Lower deadline score = higher urgency, so invert
        priority_score - deadline_score.min(10000)
    }
}

impl Ord for Task {
    fn cmp(&self, other: &Self) -> Ordering {
        // Higher effective priority first (BinaryHeap is max-heap)
        self.effective_priority().cmp(&other.effective_priority())
            .then_with(|| other.id.cmp(&self.id)) // tie: lower id first
    }
}

impl PartialOrd for Task {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A priority queue for tasks with ternary signals.
#[derive(Debug, Clone)]
pub struct TernaryPriorityQueue {
    heap: BinaryHeap<Task>,
    signal_overrides: HashMap<usize, TernaryDecision>,
}

impl TernaryPriorityQueue {
    pub fn new() -> Self {
        TernaryPriorityQueue {
            heap: BinaryHeap::new(),
            signal_overrides: HashMap::new(),
        }
    }

    /// Push a task into the queue.
    pub fn push(&mut self, task: Task) {
        self.heap.push(task);
    }

    /// Apply a ternary signal override to a task by id.
    pub fn set_signal(&mut self, task_id: usize, signal: TernaryDecision) {
        self.signal_overrides.insert(task_id, signal);
    }

    /// Pop the highest priority task, applying any signal overrides.
    pub fn pop(&mut self) -> Option<Task> {
        let mut task = self.heap.pop()?;
        if let Some(signal) = self.signal_overrides.get(&task.id) {
            task.ternary_signal = *signal;
        }
        Some(task)
    }

    /// Peek at the highest priority task.
    pub fn peek(&self) -> Option<&Task> {
        self.heap.peek()
    }

    /// Number of tasks.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    /// Drain all tasks in priority order.
    pub fn drain_sorted(&mut self) -> Vec<Task> {
        let mut tasks = Vec::new();
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

/// Deadline-aware scheduler.
#[derive(Debug, Clone)]
pub struct DeadlineScheduler {
    tasks: Vec<Task>,
    current_time: u64,
}

impl DeadlineScheduler {
    pub fn new(current_time: u64) -> Self {
        DeadlineScheduler {
            tasks: Vec::new(),
            current_time,
        }
    }

    /// Add a task.
    pub fn add(&mut self, task: Task) {
        self.tasks.push(task);
    }

    /// Advance time.
    pub fn advance_time(&mut self, dt: u64) {
        self.current_time += dt;
    }

    /// Get tasks that are overdue.
    pub fn overdue(&self) -> Vec<&Task> {
        self.tasks.iter().filter(|t| {
            t.deadline.map_or(false, |dl| dl <= self.current_time)
        }).collect()
    }

    /// Get tasks sorted by urgency (most urgent first).
    pub fn schedule_by_urgency(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.iter().collect();
        tasks.sort_by(|a, b| {
            b.urgency(self.current_time).cmp(&a.urgency(self.current_time))
        });
        tasks
    }

    /// Get tasks sorted by deadline (earliest first).
    pub fn schedule_by_deadline(&self) -> Vec<&Task> {
        let mut tasks: Vec<&Task> = self.tasks.iter().collect();
        tasks.sort_by(|a, b| {
            let da = a.deadline.unwrap_or(u64::MAX);
            let db = b.deadline.unwrap_or(u64::MAX);
            da.cmp(&db).then_with(|| b.effective_priority().cmp(&a.effective_priority()))
        });
        tasks
    }

    /// Remove and return a task by id.
    pub fn remove(&mut self, id: usize) -> Option<Task> {
        if let Some(pos) = self.tasks.iter().position(|t| t.id == id) {
            Some(self.tasks.remove(pos))
        } else {
            None
        }
    }

    /// Number of tasks.
    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Get current time.
    pub fn current_time(&self) -> u64 {
        self.current_time
    }

    /// Apply ternary decisions to modify task priorities.
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

/// Round-robin scheduler with ternary weighting.
#[derive(Debug, Clone)]
pub struct RoundRobinScheduler {
    queues: [Vec<Task>; 3], // [Prioritize, Neutral, Defer]
    position: usize,
}

impl RoundRobinScheduler {
    pub fn new() -> Self {
        RoundRobinScheduler {
            queues: [Vec::new(), Vec::new(), Vec::new()],
            position: 0,
        }
    }

    /// Add a task to the appropriate queue.
    pub fn add(&mut self, task: Task) {
        let idx = match task.ternary_signal {
            TernaryDecision::Prioritize => 0,
            TernaryDecision::Neutral => 1,
            TernaryDecision::Defer => 2,
        };
        self.queues[idx].push(task);
    }

    /// Get the next task, favoring prioritize queue.
    /// Weights: prioritize gets 3 slots, neutral 2, defer 1.
    pub fn next(&mut self) -> Option<Task> {
        let order = [0, 0, 0, 1, 1, 2]; // weighted round-robin
        for _ in 0..6 {
            let queue_idx = order[self.position % 6];
            self.position += 1;
            if !self.queues[queue_idx].is_empty() {
                return Some(self.queues[queue_idx].remove(0));
            }
        }
        // Fallback: try all queues
        for q in &mut self.queues {
            if !q.is_empty() {
                return Some(q.remove(0));
            }
        }
        None
    }

    /// Total tasks across all queues.
    pub fn len(&self) -> usize {
        self.queues.iter().map(|q| q.len()).sum()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.queues.iter().all(|q| q.is_empty())
    }

    /// Tasks in prioritize queue.
    pub fn prioritize_count(&self) -> usize {
        self.queues[0].len()
    }

    /// Tasks in defer queue.
    pub fn defer_count(&self) -> usize {
        self.queues[2].len()
    }
}

impl Default for RoundRobinScheduler {
    fn default() -> Self {
        Self::new()
    }
}

/// Schedule tasks to minimize total weighted completion time.
pub fn schedule_min_weighted_completion(tasks: &[Task]) -> Vec<usize> {
    let mut indexed: Vec<(usize, &Task)> = tasks.iter().enumerate().collect();
    // Sort by effective priority descending (higher priority = earlier)
    indexed.sort_by(|a, b| b.1.effective_priority().cmp(&a.1.effective_priority()));
    indexed.iter().map(|(i, _)| *i).collect()
}

/// Schedule tasks respecting deadlines using earliest deadline first.
/// Returns task indices in order, or None if infeasible.
pub fn earliest_deadline_first(tasks: &[Task]) -> Option<Vec<usize>> {
    let mut indexed: Vec<(usize, &Task)> = tasks.iter().enumerate().collect();
    // Sort by deadline ascending
    indexed.sort_by(|a, b| {
        let da = a.1.deadline.unwrap_or(u64::MAX);
        let db = b.1.deadline.unwrap_or(u64::MAX);
        da.cmp(&db)
    });

    let mut time: u64 = 0;
    let mut result = Vec::new();
    for (orig_idx, task) in &indexed {
        time += task.effort as u64;
        if let Some(dl) = task.deadline {
            if time > dl {
                return None; // infeasible
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
        assert_eq!(TernaryDecision::from_value(1), Some(TernaryDecision::Prioritize));
        assert_eq!(TernaryDecision::from_value(-1), Some(TernaryDecision::Defer));
        assert_eq!(TernaryDecision::from_value(0), Some(TernaryDecision::Neutral));
        assert_eq!(TernaryDecision::from_value(5), None);
    }

    #[test]
    fn test_task_effective_priority() {
        let t = Task::new(1, "test").with_priority(5).with_signal(TernaryDecision::Prioritize);
        assert_eq!(t.effective_priority(), 6);
    }

    #[test]
    fn test_task_effective_priority_defer() {
        let t = Task::new(1, "test").with_priority(5).with_signal(TernaryDecision::Defer);
        assert_eq!(t.effective_priority(), 4);
    }

    #[test]
    fn test_priority_queue_ordering() {
        let mut pq = TernaryPriorityQueue::new();
        pq.push(Task::new(1, "low").with_priority(1));
        pq.push(Task::new(2, "high").with_priority(10));
        pq.push(Task::new(3, "mid").with_priority(5));

        let tasks = pq.drain_sorted();
        // Ord: higher effective_priority first; on tie, lower id first
        assert_eq!(tasks[0].id, 2); // priority 10
        assert_eq!(tasks[1].id, 3); // priority 5
        assert_eq!(tasks[2].id, 1); // priority 1
    }

    #[test]
    fn test_priority_queue_with_signals() {
        let mut pq = TernaryPriorityQueue::new();
        pq.push(Task::new(1, "a").with_priority(5).with_signal(TernaryDecision::Defer));
        pq.push(Task::new(2, "b").with_priority(3).with_signal(TernaryDecision::Prioritize));

        let tasks = pq.drain_sorted();
        // Task 1: 5-1=4, Task 2: 3+1=4, tied effective priority, lower id first
        assert_eq!(tasks[0].id, 1);
        assert_eq!(tasks[1].id, 2);
    }

    #[test]
    fn test_priority_queue_signal_override() {
        let mut pq = TernaryPriorityQueue::new();
        pq.push(Task::new(1, "a").with_priority(5));
        pq.push(Task::new(2, "b").with_priority(3));
        pq.set_signal(2, TernaryDecision::Prioritize);

        let first = pq.pop().unwrap();
        // Task 2 with override: 3+1=4, Task 1: 5, task 1 wins
        // But signal override is applied at pop, heap was built without override
        // The heap ordering is based on original Task ord (base_priority=5 vs 3)
        assert_eq!(first.id, 1); // priority 5 > 3
    }

    #[test]
    fn test_priority_queue_empty() {
        let mut pq = TernaryPriorityQueue::new();
        assert!(pq.is_empty());
        assert!(pq.pop().is_none());
    }

    #[test]
    fn test_deadline_scheduler_overdue() {
        let mut ds = DeadlineScheduler::new(100);
        ds.add(Task::new(1, "late").with_deadline(50));
        ds.add(Task::new(2, "ontime").with_deadline(200));

        let overdue = ds.overdue();
        assert_eq!(overdue.len(), 1);
        assert_eq!(overdue[0].id, 1);
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
    }

    #[test]
    fn test_deadline_scheduler_apply_decisions() {
        let mut ds = DeadlineScheduler::new(0);
        ds.add(Task::new(1, "a").with_priority(5));
        ds.apply_decisions(&[(1, TernaryDecision::Prioritize)]);
        assert_eq!(ds.tasks[0].ternary_signal, TernaryDecision::Prioritize);
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

        let mut ids = Vec::new();
        while let Some(t) = rr.next() {
            ids.push(t.id);
        }
        assert_eq!(ids.len(), 4);
        // Prioritize tasks should come first
        assert!(ids[0] == 1 || ids[0] == 2);
    }

    #[test]
    fn test_round_robin_empty() {
        let mut rr = RoundRobinScheduler::new();
        assert!(rr.is_empty());
        assert!(rr.next().is_none());
    }

    #[test]
    fn test_min_weighted_completion() {
        let tasks = vec![
            Task::new(0, "low").with_priority(1),
            Task::new(1, "high").with_priority(10),
            Task::new(2, "mid").with_priority(5),
        ];
        let order = schedule_min_weighted_completion(&tasks);
        assert_eq!(order[0], 1); // high priority first
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
        let tasks = vec![
            Task::new(0, "long").with_deadline(5).with_effort(10),
        ];
        assert!(earliest_deadline_first(&tasks).is_none());
    }

    #[test]
    fn test_earliest_deadline_no_deadlines() {
        let tasks = vec![
            Task::new(0, "a"),
            Task::new(1, "b"),
        ];
        assert!(earliest_deadline_first(&tasks).is_some());
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
