//! Runnable mirror of the README "Quick Start".
//!
//! This example exists so the README's claims are continuously verified: if the
//! public API changes in a way that breaks the documented behaviour, `cargo
//! run --example quickstart` (and the example's assertions) fail.

use ternary_scheduling::*;

fn main() {
    // Priority queue with ternary signals
    let mut pq = TernaryPriorityQueue::new();
    pq.push(
        Task::new(1, "urgent_report")
            .with_priority(5)
            .with_signal(TernaryDecision::Prioritize),
    );
    pq.push(
        Task::new(2, "daily_sync")
            .with_priority(3)
            .with_signal(TernaryDecision::Neutral),
    );
    pq.push(
        Task::new(3, "backlog_cleanup")
            .with_priority(1)
            .with_signal(TernaryDecision::Defer),
    );

    let tasks = pq.drain_sorted();
    assert_eq!(tasks[0].name, "urgent_report"); // effective priority: 6
    assert_eq!(tasks[1].name, "daily_sync"); // effective priority: 3
    assert_eq!(tasks[2].name, "backlog_cleanup"); // effective priority: 0

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

    println!("Quick Start example: all assertions held.");
}
