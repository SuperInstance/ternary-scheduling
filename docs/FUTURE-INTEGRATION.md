# Future Integration: ternary-scheduling

## Current State
Provides task scheduling with ternary decisions: priority queues with `TernaryDecision` signals (Prioritize/Defer/Neutral), deadline-aware scheduling, effort estimation, and weighted task ordering. Standard library (`std`) based with `BinaryHeap`-backed priority queues.

## Integration Opportunities

### With ternary-cell (Tick Cycle Scheduling)
ternary-cell's six-phase tick (predict→perceive→surprise→vibe→gc→conservation) needs scheduling when multiple cells compete for compute. ternary-scheduling assigns `TernaryDecision` to each cell based on surprise magnitude: high-surprise cells get Prioritize, low-surprise get Defer, near-prediction get Neutral. Deadlines correspond to the tick cycle interval — a cell must complete its tick before the next global sync.

### With ternary-signals (Frequency-Driven Scheduling)
Spectral analysis from ternary-signals drives scheduling priority. Cells oscillating at high frequency (unstable) get Prioritize. Cells at low frequency (stable) get Defer. The `Task::effort` field maps to the cell's grid size — larger subgrids need more compute and are scheduled accordingly.

### With construct-core (Skill Loading Priority)
construct-core's `load_skill()`/`unload_skill()` needs scheduling when multiple skills are requested simultaneously. ternary-scheduling orders skill loading by ternary signal (skill urgency), deadline (task completion deadline), and effort (skill size in memory). The `HashMap`-backed task registry maps skill IDs to scheduling metadata.

## Potential in Mature Systems
In room-as-codespace, PLATO manages hundreds of rooms (Codespaces) that need startup ordering, resource allocation, and priority arbitration. ternary-scheduling becomes the room scheduler: each room is a `Task` with ternary urgency (user waiting = Prioritize, background = Defer), deadline (SLA commitments), and effort (Codespace spin-up time ~2-3 minutes). The scheduler ensures high-priority rooms get Codespace allocation first when compute is constrained.

## Cross-Pollination Ideas
- **ternary-pareto**: Multi-objective scheduling — optimize for both latency and throughput simultaneously, with Pareto-optimal schedules that can't be improved on one objective without hurting another.
- **ternary-games**: Game-theoretic scheduling — rooms compete for shared compute. Nash equilibrium scheduling ensures no room wants to "deviate" (request different resources).
- **ternary-curriculum**: Progressive scheduling — easy rooms (lightweight skills) are scheduled first as "warm-up," building to complex rooms.

## Dependencies for Next Steps
- Define `RoomScheduler` trait in PLATO wrapping ternary-scheduling
- Add `TernaryDecision` computation from cell surprise values
- Integrate with construct-core's skill loading for priority-based `load_skill()`
- Consider `no_std` subset (heap-based scheduling → fixed-size priority queue)
