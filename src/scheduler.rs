//! Layer-2 task timer — the DAG scheduler (§6.2). Pure data structure: it
//! computes WHO MAY RUN; it never spawns, never touches I/O. The orchestrator
//! (Faz 4-c) drives it. That split keeps the whole §6.2 rule set
//! (topological admission, worker-pool cap, single-writer serialization,
//! blocked propagation, critical-path recompute) exhaustively unit-testable,
//! and lets resume rebuild the exact same scheduler from `tasks` statuses.
//!
//! Plan rules, one paragraph each:
//! - `parallel_groups` is INFORMATIONAL — admission follows `dependencies`.
//! - A task goes `in_progress` only when ALL deps are `done`.
//! - Single writer: tasks whose artifact paths intersect are serialized even
//!   if the DAG says they are parallel.
//! - Terminal states `done|failed|skipped|blocked` never trigger the FSM
//!   directly; the only signal is `all_terminal()` (⇒ AllTasksTerminal).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;

use crate::models::task::{TaskItem, TaskPlan};

/// `tasks.status` — mirrors the schema enum exactly (§11: schema is truth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    InProgress,
    Done,
    Failed,
    Skipped,
    Blocked,
}

impl Status {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "pending" => Self::Pending,
            "in_progress" => Self::InProgress,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            "blocked" => Self::Blocked,
            _ => return None,
        })
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Blocked => "blocked",
        }
    }
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Done | Self::Failed | Self::Skipped | Self::Blocked
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchedError {
    /// §6.2: a task depends on an id that is not in the plan.
    UnknownDependency {
        task: String,
        dep: String,
    },
    /// §21.2 unit scope: "DAG zamanlayıcı (döngü, ...)".
    Cycle(Vec<String>),
    /// Duplicate ids would make the PK (spec_id, task_id) lie.
    DuplicateTask(String),
    StatusParse(String),
}

impl std::fmt::Display for SchedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownDependency { task, dep } => {
                write!(f, "DAG: {task} depends on unknown task {dep}")
            }
            Self::Cycle(members) => write!(f, "DAG: dependency cycle through {members:?}"),
            Self::DuplicateTask(id) => write!(f, "DAG: duplicate task id {id}"),
            Self::StatusParse(s) => write!(f, "DAG: unknown task status {s:?}"),
        }
    }
}
impl std::error::Error for SchedError {}

/// Normalize an artifact path for ownership comparison (§12.2: never hardcode
/// separators). `\` → `/`, drop `./`, strip leading `**`. Case is PRESERVED —
/// on case-sensitive file systems `Mod.rs` and `mod.rs` are different owners.
fn norm_path(p: &str) -> String {
    let mut s: String = p.chars().map(|c| if c == '\\' { '/' } else { c }).collect();
    while let Some(rest) = s.strip_prefix("./") {
        s = rest.to_string();
    }
    if let Some(rest) = s.strip_prefix('/') {
        s = rest.to_string();
    }
    s.replace("/**", "").replace("**", "")
}

fn owned_paths(t: &TaskItem) -> BTreeSet<String> {
    t.file_ownership
        .iter()
        .chain(t.artifacts_expected.iter())
        .map(|p| norm_path(p))
        .filter(|p| !p.is_empty())
        .collect()
}

#[derive(Debug, Clone)]
struct Node {
    deps: BTreeSet<String>,
    /// Reverse edges for blocked propagation, sorted for determinism.
    dependents: Vec<String>,
    paths: BTreeSet<String>,
    status: Status,
    retry: u32,
}

/// What `admit()` decided, with the reason each deferred task waited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Admission {
    /// Ordered by id — deterministic under any input order (§21.1).
    pub admitted: Vec<String>,
    /// (waiting task, running task that owns the colliding path) — the
    /// orchestrator records `TaskOwnershipConflict` for these pairs.
    pub deferred_conflict: Vec<(String, String)>,
    /// Ready tasks held back purely by the worker-pool cap.
    pub deferred_cap: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Scheduler {
    nodes: BTreeMap<String, Node>,
    max_concurrency: usize,
}

impl Scheduler {
    pub fn from_plan(plan: &TaskPlan, max_concurrency: u32) -> Result<Self, SchedError> {
        let mut nodes = BTreeMap::new();
        for t in &plan.tasks {
            let status = Status::parse(&t.status)
                .ok_or_else(|| SchedError::StatusParse(format!("{}: {:?}", t.id, t.status)))?;
            if nodes
                .insert(
                    t.id.clone(),
                    Node {
                        deps: t.dependencies.iter().cloned().collect(),
                        dependents: Vec::new(),
                        paths: owned_paths(t),
                        status,
                        retry: t.retry_count,
                    },
                )
                .is_some()
            {
                return Err(SchedError::DuplicateTask(t.id.clone()));
            }
        }
        for (id, n) in &nodes {
            for d in &n.deps {
                if !nodes.contains_key(d) {
                    return Err(SchedError::UnknownDependency {
                        task: id.clone(),
                        dep: d.clone(),
                    });
                }
            }
        }
        // Build reverse edges after validation so a bad graph never half-wires.
        let ids: Vec<String> = nodes.keys().cloned().collect();
        for id in &ids {
            let deps: Vec<String> = nodes[id].deps.iter().cloned().collect();
            for d in deps {
                nodes.get_mut(&d).unwrap().dependents.push(id.clone());
            }
        }
        let s = Self {
            nodes,
            max_concurrency: (max_concurrency as usize).max(1),
        };
        // Self-dependency is the smallest cycle; Kahn catches it too.
        s.check_acyclic()?;
        Ok(s)
    }

    /// Kahn's algorithm: deterministic (BTreeMap order), and the leftovers are
    /// exactly the cycle members.
    fn check_acyclic(&self) -> Result<(), SchedError> {
        let mut indeg: HashMap<&str, usize> = HashMap::new();
        for (id, n) in &self.nodes {
            indeg.insert(id.as_str(), n.deps.len());
        }
        let mut queue: Vec<&str> = indeg
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(k, _)| *k)
            .collect();
        queue.sort();
        let mut seen = 0usize;
        while let Some(id) = queue.pop() {
            seen += 1;
            let mut next: Vec<&str> = Vec::new();
            for dep in &self.nodes[id].dependents {
                let e = indeg.get_mut(dep.as_str()).unwrap();
                *e -= 1;
                if *e == 0 {
                    next.push(dep.as_str());
                }
            }
            next.sort();
            queue.extend(next);
        }
        if seen == self.nodes.len() {
            Ok(())
        } else {
            let mut cyc: Vec<String> = self
                .nodes
                .iter()
                .filter(|(id, _)| indeg[id.as_str()] > 0)
                .map(|(id, _)| id.clone())
                .collect();
            cyc.sort();
            Err(SchedError::Cycle(cyc))
        }
    }

    pub fn status(&self, id: &str) -> Option<Status> {
        self.nodes.get(id).map(|n| n.status)
    }

    pub fn paths(&self, id: &str) -> Option<&BTreeSet<String>> {
        self.nodes.get(id).map(|n| &n.paths)
    }

    pub fn running_ids(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.status == Status::InProgress)
            .map(|(id, _)| id.clone())
            .collect()
    }

    fn deps_done(&self, n: &Node) -> bool {
        n.deps.iter().all(|d| self.nodes[d].status == Status::Done)
    }

    /// §6.2 — ready set: pending, deps all done, in id order.
    pub fn ready(&self) -> Vec<String> {
        self.nodes
            .iter()
            .filter(|(_, n)| n.status == Status::Pending && self.deps_done(n))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// The admission decision for one scheduler tick: filter `ready()` by
    /// remaining pool slots and by the single-writer rule (a ready task whose
    /// owned paths intersect ANY in-progress task's paths waits for that task
    /// instead of racing it).
    pub fn admit(&self) -> Admission {
        let running = self.running_ids();
        let slots = self.max_concurrency.saturating_sub(running.len());
        let mut out = Admission {
            admitted: Vec::new(),
            deferred_conflict: Vec::new(),
            deferred_cap: Vec::new(),
        };
        let mut reserved: Vec<(String, &BTreeSet<String>)> = running
            .iter()
            .map(|id| (id.clone(), &self.nodes[id].paths))
            .collect();
        let mut started = 0usize;
        for id in self.ready() {
            let paths = &self.nodes[&id].paths;
            let conflict = reserved
                .iter()
                .find(|(_, owned)| owned.iter().any(|p| paths.contains(p)));
            if let Some((with, _)) = conflict {
                out.deferred_conflict.push((id.clone(), with.clone()));
            } else if started >= slots {
                out.deferred_cap.push(id.clone());
            } else {
                out.admitted.push(id.clone());
                reserved.push((id.clone(), paths));
                started += 1;
            }
        }
        out
    }

    pub fn begin(&mut self, id: &str) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.status = Status::InProgress;
        }
    }

    pub fn set_status(&mut self, id: &str, s: Status) {
        if let Some(n) = self.nodes.get_mut(id) {
            n.status = s;
        }
    }

    /// Mark a task terminal (non-done) and cascade `Blocked` through the
    /// pending descendants — anything that can no longer reach `all deps
    /// done` is blocked by definition (§6.2 terminal-state closure). Returns
    /// the newly-blocked ids (id order) so the caller can persist them — the
    /// DB, not memory, is the single source of truth (§7.1) a resume rebuilds
    /// from, so the cascade MUST reach `tasks`.
    pub fn mark_unreachable(&mut self, id: &str, final_status: Status) -> Vec<String> {
        debug_assert!(final_status.is_terminal() && final_status != Status::Done);
        self.set_status(id, final_status);
        let mut frontier: Vec<String> = self
            .nodes
            .get(id)
            .map(|n| n.dependents.clone())
            .unwrap_or_default();
        let mut blocked = Vec::new();
        while let Some(dep) = frontier.pop() {
            // Borrow the list out before mutating: in-progress work is NOT
            // cancelled mid-flight; a pending task whose dep chain is now
            // dead blocks immediately (§6.2 terminal-state closure).
            let child_dependents = match self.nodes[&dep].status {
                Status::Pending => self.nodes[&dep].dependents.clone(),
                _ => continue, // InProgress survives; terminal already handled
            };
            self.nodes.get_mut(&dep).unwrap().status = Status::Blocked;
            blocked.push(dep);
            frontier.extend(child_dependents);
        }
        blocked.sort();
        blocked
    }

    /// §6.2 — the ONLY FSM signal from layer 2: everything terminal, and at
    /// least one `done` (a run where nothing succeeded must not "finalize"
    /// into a report of success).
    pub fn all_terminal(&self) -> bool {
        !self.nodes.is_empty()
            && self.nodes.values().all(|n| n.status.is_terminal())
            && self.nodes.values().any(|n| n.status == Status::Done)
    }

    /// Deadlock guard: nothing running, nothing admissible, not terminal.
    /// (Blocked-cascade makes this rare, but the orchestrator must not spin.)
    pub fn stalled(&self) -> bool {
        !self.all_terminal() && self.running_ids().is_empty() && self.ready().is_empty()
    }

    pub fn statuses(&self) -> BTreeMap<String, (Status, u32)> {
        self.nodes
            .iter()
            .map(|(id, n)| (id.clone(), (n.status, n.retry)))
            .collect()
    }

    /// The `status_summary` DERIVED from live node states. §7.1 single-truth:
    /// the plan document's declared summary is A2's snapshot at authoring time
    /// and may be stale (resume reloads tasks with live statuses); any
    /// summary the control plane reports must come from here, never from the
    /// input file's copy.
    pub fn status_summary(&self) -> crate::models::task::StatusSummary {
        let mut s = crate::models::task::StatusSummary {
            total: self.nodes.len() as u32,
            ..Default::default()
        };
        for n in self.nodes.values() {
            match n.status {
                Status::Pending => s.pending += 1,
                Status::InProgress => s.in_progress += 1,
                Status::Done => s.done += 1,
                Status::Failed => s.failed += 1,
                Status::Skipped => s.skipped += 1,
                Status::Blocked => {} // schema StatusSummary has no blocked field (§11.2)
            }
        }
        s
    }

    /// Deterministic full topological order (Kahn, id tie-break). Used by
    /// resume replay and to validate A2's suggested `execution_order`.
    pub fn topological_order(&self) -> Vec<String> {
        let mut indeg: HashMap<&str, usize> = self
            .nodes
            .iter()
            .map(|(id, n)| (id.as_str(), n.deps.len()))
            .collect();
        let mut done: std::collections::HashSet<&str> = std::collections::HashSet::new();
        let mut out = Vec::with_capacity(self.nodes.len());
        loop {
            let mut avail: Vec<&str> = indeg
                .iter()
                .filter(|(k, d)| **d == 0 && !done.contains(*k))
                .map(|(k, _)| *k)
                .collect();
            if avail.is_empty() {
                return out;
            }
            avail.sort();
            let pick = avail.remove(0);
            done.insert(pick);
            out.push(pick.to_string());
            for dep in &self.nodes[pick].dependents {
                let e = indeg.get_mut(dep.as_str()).unwrap();
                *e = e.saturating_sub(1);
            }
        }
    }

    /// Longest dependency chain (in nodes). Among equal-length candidates the
    /// lexicographically smallest path is chosen, so the result is stable and
    /// comparable in golden tests — this is the scheduler's REALITY for §6.2's
    /// "kritik_path öneridir; doğrulanmazsa zamanlayıcı yeniden hesaplar".
    pub fn critical_path(&self) -> Vec<String> {
        let order = self.topological_order();
        // len[v] = longest chain starting at v; next[v] = successor to take.
        let mut len: HashMap<&str, usize> = HashMap::new();
        let mut next: HashMap<&str, String> = HashMap::new();
        for id in order.iter().rev() {
            let mut best: (usize, String) = (1, String::new());
            for dep in &self.nodes[id].dependents {
                let l = len.get(dep.as_str()).copied().unwrap_or(1) + 1;
                // strictly longer wins; on tie the lex-smaller successor wins
                if l > best.0 || (l == best.0 && dep.as_str() < best.1.as_str()) {
                    best = (l, dep.clone());
                }
            }
            len.insert(id.as_str(), best.0);
            if !best.1.is_empty() {
                next.insert(id.as_str(), best.1);
            }
        }
        let max_len = len.values().copied().max().unwrap_or(1);
        let start: String = order
            .iter()
            .filter(|id| len[id.as_str()] == max_len)
            .min()
            .cloned()
            .unwrap_or_default();
        let mut path = Vec::new();
        let mut cur = start;
        while !cur.is_empty() {
            path.push(cur.clone());
            cur = next.get(cur.as_str()).cloned().unwrap_or_default();
        }
        path
    }

    /// Accept A2's proposal only if it IS a critical path: consecutive ids are
    /// real edges, no branches skipped, length equals the recomputed longest.
    /// Otherwise return the recomputed one (flagged so the orchestrator can
    /// log "öneri doğrulanamadı").
    pub fn validate_critical_path(&self, proposed: &[String]) -> (Vec<String>, bool) {
        let truth = self.critical_path();
        let valid = proposed.len() == truth.len()
            && proposed.windows(2).all(|w| {
                self.nodes
                    .get(&w[1])
                    .is_some_and(|n| n.deps.contains(&w[0]))
            });
        if valid {
            (proposed.to_vec(), true)
        } else {
            (truth, false)
        }
    }
}

/// `payload` for an agent_call op (§7.4: resume retries "aynı prompt+girdi
/// hash'i ile"). Everything that can change behavior on re-run must be in.
pub fn task_payload(t: &TaskItem) -> Value {
    serde_json::json!({
        "task_id": t.id,
        "agent": t.assigned_agent,
        "title": t.title,
        "retry": t.retry_count,
        "acceptance": t.acceptance.criteria,
        "deps_done": true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::task::{Acceptance, TaskItem, TaskPlan};

    fn task(id: &str, deps: &[&str], paths: &[&str], status: &str) -> TaskItem {
        TaskItem {
            id: id.into(),
            spec_requirement_ids: vec!["R1".into()],
            title: format!("task {id}"),
            description: String::new(),
            department: None,
            assigned_agent: "A5".into(),
            dependencies: deps.iter().map(|s| s.to_string()).collect(),
            target: "cloud".into(),
            toolchain: Some("rust".into()),
            acceptance: Acceptance {
                verifiable_by: "A4".into(),
                criteria: "ok".into(),
            },
            estimated: None,
            difficulty: None,
            file_ownership: paths.iter().map(|s| s.to_string()).collect(),
            artifacts_expected: paths.iter().map(|s| s.to_string()).collect(),
            artifacts_produced: vec![],
            blocked_by: vec![],
            status: status.into(),
            retry_count: 0,
            escalation_level: 0,
            notes: String::new(),
            extra: Default::default(),
        }
    }

    fn plan(tasks: Vec<TaskItem>) -> TaskPlan {
        TaskPlan {
            schema_version: "1.0".into(),
            spec_id: "spec-test".into(),
            created_at: "2026-09-12T00:00:00Z".into(),
            created_by: "A2".into(),
            revision: 1,
            tasks,
            execution_order: vec![],
            parallel_groups: vec![],
            critical_path: vec![],
            status_summary: Default::default(),
            extra: Default::default(),
        }
    }

    fn sched(tasks: Vec<TaskItem>, cap: u32) -> Scheduler {
        Scheduler::from_plan(&plan(tasks), cap).unwrap()
    }

    #[test]
    fn linear_chain_admits_one_at_a_time_in_order() {
        let mut s = sched(
            vec![
                task("T1", &[], &["a.rs"], "pending"),
                task("T2", &["T1"], &["b.rs"], "pending"),
                task("T3", &["T2"], &["c.rs"], "pending"),
            ],
            2,
        );
        assert_eq!(s.ready(), vec!["T1"]);
        assert_eq!(s.admit().admitted, vec!["T1"]);
        s.begin("T1");
        assert!(s.admit().admitted.is_empty());
        s.set_status("T1", Status::Done);
        assert_eq!(s.admit().admitted, vec!["T2"]);
        assert_eq!(s.topological_order(), vec!["T1", "T2", "T3"]);
    }

    #[test]
    fn parallel_group_admits_both_within_cap_then_defers_third_by_cap() {
        let mut s = sched(
            vec![
                task("T1", &[], &["src/a.rs"], "pending"),
                task("T2", &[], &["src/b.rs"], "pending"),
                task("T3", &[], &["src/c.rs"], "pending"),
            ],
            2,
        );
        let a = s.admit();
        assert_eq!(a.admitted, vec!["T1", "T2"], "cap=2 admits two");
        assert_eq!(a.deferred_cap, vec!["T3"]);
        for t in a.admitted {
            s.begin(&t);
        }
        assert!(s.admit().admitted.is_empty(), "pool full");
        s.set_status("T1", Status::Done);
        assert_eq!(s.admit().admitted, vec!["T3"], "slot freed in id order");
    }

    #[test]
    fn file_ownership_conflict_serializes_even_when_dag_allows_parallel() {
        let mut s = sched(
            vec![
                task("T1", &[], &["src/lib.rs"], "pending"),
                task("T2", &[], &["src/lib.rs"], "pending"),
                task("T3", &[], &["src/other.rs"], "pending"),
            ],
            8,
        );
        let a = s.admit();
        assert_eq!(a.admitted, vec!["T1", "T3"], "T2 collides on src/lib.rs");
        assert_eq!(a.deferred_conflict, vec![("T2".into(), "T1".into())]);
        for t in a.admitted {
            s.begin(&t);
        }
        s.set_status("T1", Status::Done);
        s.set_status("T3", Status::Done);
        assert_eq!(
            s.admit().admitted,
            vec!["T2"],
            "colliding task admitted once owner finishes"
        );
    }

    #[test]
    fn path_normalization_catches_windows_separators_and_dot_prefix() {
        let mut s = sched(
            vec![
                task("T1", &[], &["src\\routes.rs", "./src/main.rs"], "pending"),
                task("T2", &[], &["src/routes.rs", "src/main.rs"], "pending"),
            ],
            4,
        );
        let a = s.admit();
        assert_eq!(a.admitted, vec!["T1"]);
        assert_eq!(a.deferred_conflict, vec![("T2".into(), "T1".into())]);
        s.begin("T1");
    }

    #[test]
    fn cycle_is_detected_not_deadlocked() {
        let e = Scheduler::from_plan(
            &plan(vec![
                task("T1", &["T2"], &[], "pending"),
                task("T2", &["T1"], &[], "pending"),
            ]),
            2,
        )
        .unwrap_err();
        assert_eq!(e, SchedError::Cycle(vec!["T1".into(), "T2".into()]));
        // self-loop
        let e =
            Scheduler::from_plan(&plan(vec![task("T1", &["T1"], &[], "pending")]), 2).unwrap_err();
        assert!(matches!(e, SchedError::Cycle(_)));
    }

    #[test]
    fn unknown_dependency_is_rejected() {
        let e =
            Scheduler::from_plan(&plan(vec![task("T1", &["T7"], &[], "pending")]), 2).unwrap_err();
        assert_eq!(
            e,
            SchedError::UnknownDependency {
                task: "T1".into(),
                dep: "T7".into()
            }
        );
    }

    #[test]
    fn duplicate_task_id_is_rejected() {
        let e = Scheduler::from_plan(
            &plan(vec![
                task("T1", &[], &[], "pending"),
                task("T1", &[], &[], "pending"),
            ]),
            2,
        )
        .unwrap_err();
        assert_eq!(e, SchedError::DuplicateTask("T1".into()));
    }

    #[test]
    fn failure_blocks_pending_descendants_transitively() {
        let mut s = sched(
            vec![
                task("A", &[], &[], "pending"),
                task("B", &["A"], &[], "pending"),
                task("C", &["B"], &[], "pending"),
                task("D", &["A"], &[], "pending"),
                task("E", &[], &[], "pending"),
            ],
            2,
        );
        s.begin("A");
        s.begin("E");
        s.set_status("E", Status::Done);
        s.mark_unreachable("A", Status::Failed);
        assert_eq!(s.status("B"), Some(Status::Blocked));
        assert_eq!(
            s.status("C"),
            Some(Status::Blocked),
            "cascade must follow the chain"
        );
        assert_eq!(s.status("D"), Some(Status::Blocked));
        // E done, A failed, B/C/D blocked → not all-done, but all terminal
        assert!(s.all_terminal());
    }

    #[test]
    fn in_progress_work_is_not_cancelled_by_ancestor_failure() {
        let mut s = sched(
            vec![
                task("A", &[], &[], "pending"),
                task("B", &[], &[], "pending"),
                task("C", &["A", "B"], &[], "pending"),
            ],
            4,
        );
        for t in ["A", "B"] {
            s.begin(t);
        }
        s.mark_unreachable("A", Status::Failed);
        assert_eq!(
            s.status("B"),
            Some(Status::InProgress),
            "running work survives"
        );
        s.set_status("B", Status::Skipped);
        assert_eq!(s.status("C"), Some(Status::Blocked));
    }

    #[test]
    fn all_terminal_requires_at_least_one_done() {
        // §6.2: "tümü son durumda + en az bir done".
        let mut s = sched(
            vec![
                task("A", &[], &[], "pending"),
                task("B", &["A"], &[], "pending"),
            ],
            2,
        );
        s.mark_unreachable("A", Status::Failed);
        assert_eq!(s.status("B"), Some(Status::Blocked));
        assert!(
            !s.all_terminal(),
            "A failed + B blocked: all terminal but NO done"
        );
        // A failed, B blocked: NO done → the signal must not fire.
        let s2 = sched(
            vec![
                task("A", &[], &[], "failed"),
                task("B", &["A"], &[], "blocked"),
            ],
            2,
        );
        assert!(!s2.all_terminal());
        let s3 = sched(
            vec![
                task("A", &[], &[], "done"),
                task("B", &["A"], &[], "blocked"),
            ],
            2,
        );
        assert!(s3.all_terminal());
    }

    #[test]
    fn resume_rebuilds_from_persisted_statuses() {
        // Kill happens after T1/T2 done; reopen with the statuses the ledger
        // guarantees reached state.db and confirm only T3 remains.
        let s = sched(
            vec![
                task("T1", &[], &["a"], "done"),
                task("T2", &[], &["b"], "done"),
                task("T3", &["T1", "T2"], &["c"], "pending"),
            ],
            2,
        );
        assert_eq!(s.ready(), vec!["T3"]);
        assert_eq!(s.running_ids(), Vec::<String>::new());
    }

    #[test]
    fn status_summary_is_derived_not_declared() {
        let mut s = sched(
            vec![
                task("A", &[], &[], "pending"),
                task("B", &["A"], &[], "done"),
                task("C", &["A"], &[], "in_progress"),
                task("D", &["B"], &[], "failed"),
            ],
            2,
        );
        s.mark_unreachable("D", Status::Failed);
        let sum = s.status_summary();
        assert_eq!(sum.done, 1);
        assert_eq!(sum.in_progress, 1);
        assert_eq!(sum.failed, 1);
        assert_eq!(sum.pending, 1);
        // A has no deps and was never run ⇒ still pending. Nothing blocked
        // here (D has no dependents), so the bucketed fields sum to total.
        assert_eq!(
            sum.pending + sum.in_progress + sum.done + sum.failed + sum.skipped,
            sum.total
        );
    }

    #[test]
    fn critical_path_is_the_longest_chain() {
        let s = sched(
            vec![
                task("T1", &[], &[], "pending"),
                task("T2", &["T1"], &[], "pending"),
                task("T3", &["T1"], &[], "pending"),
                task("T4", &["T2"], &[], "pending"),
            ],
            2,
        );
        assert_eq!(s.critical_path(), vec!["T1", "T2", "T4"]);
        // valid proposal accepted verbatim
        let (path, ok) = s.validate_critical_path(&["T1".into(), "T2".into(), "T4".into()]);
        assert!(ok);
        assert_eq!(path, vec!["T1", "T2", "T4"]);
        // bogus proposal (skips an edge, wrong length) → recomputed
        let (path, ok) = s.validate_critical_path(&["T1".into(), "T3".into()]);
        assert!(!ok);
        assert_eq!(path, vec!["T1", "T2", "T4"]);
    }

    #[test]
    fn stalled_when_nothing_runs_and_nothing_admissible() {
        let mut s = sched(
            vec![
                task("A", &["B"], &[], "pending"),
                task("B", &[], &[], "blocked"),
            ],
            2,
        );
        // manually mark B blocked (not via cascade — B has no deps)
        s.set_status("B", Status::Blocked);
        assert!(s.stalled());
    }

    /// §21.2 fuzz layer: deterministic random DAGs — the scheduler must never
    /// panic, cycles must be rejected, acyclic graphs must fully drain.
    #[test]
    fn fuzz_random_dags_always_drain_or_reject_cycle() {
        let mut state = 0x5eed_u64;
        let mut rnd = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut cycles = 0;
        for _ in 0..500 {
            let n = 1 + (rnd() % 8) as usize;
            let ids: Vec<String> = (0..n).map(|i| format!("T{i}")).collect();
            let mut tasks = Vec::new();
            for (i, id) in ids.iter().enumerate() {
                let deps: Vec<&str> = (0..i)
                    .filter(|_| rnd() % 4 == 0)
                    .map(|j| ids[j].as_str())
                    .collect();
                let path = format!("f{i}.rs");
                tasks.push(task(id, &deps, &[path.as_str()], "pending"));
            }
            // add a cycle with probability ~1/4
            if rnd() % 4 == 0 && n >= 2 {
                tasks[0].dependencies.push(ids[n - 1].clone());
            }
            match Scheduler::from_plan(&plan(tasks), 1 + (rnd() % 3) as u32) {
                Err(SchedError::Cycle(_)) => cycles += 1,
                Err(e) => panic!("unexpected rejection: {e}"),
                Ok(mut s) => {
                    let mut guard = 0;
                    while !s.all_terminal() && !s.stalled() {
                        guard += 1;
                        assert!(guard < 100, "drain did not converge");
                        let a = s.admit();
                        if a.admitted.is_empty() {
                            // only cap/conflict deferrals: finish a running task
                            let r = s.running_ids();
                            assert!(!r.is_empty(), "no admitted, none running, not stalled");
                            s.set_status(&r[0], Status::Done);
                            continue;
                        }
                        for t in &a.admitted {
                            s.begin(t);
                        }
                        for t in a.admitted {
                            s.set_status(&t, Status::Done);
                        }
                    }
                }
            }
        }
        assert!(cycles > 0, "fuzz must have produced cycles to reject");
    }
}
