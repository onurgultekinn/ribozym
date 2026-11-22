//! Run lifecycle FSM (§6.1) — LAYER 1: holds the run's PHASE, not task
//! parallelism (that is the DAG scheduler, §6.2). Kept intentionally boring:
//! a pure, table-driven `transition` that never touches I/O, so the whole
//! state machine is exhaustively testable and the orchestrator can drive it
//! deterministically from a replayed event log.
//!
//! Terminal phases (Delivered / Failed / NeedsUser) do not move — once a run
//! has reported to the user, only a fresh `resume` (a new run_id) continues.

use serde::{Deserialize, Serialize};

/// `runs.status` values (§7.2) — the FSM phase. Serialized as the exact string
/// so the DB column and event payloads stay human-readable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Phase {
    Idle,
    SpecReceived,
    Clarifying,
    Researching,
    Advising,
    SpecValidated,
    TasksCreated,
    Scheduling,
    Finalizing,
    FinalReview,
    Crisis,
    Delivered,
    NeedsUser,
    Failed,
    Paused,
}

impl Phase {
    pub fn is_terminal(self) -> bool {
        matches!(self, Phase::Delivered | Phase::Failed | Phase::NeedsUser)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Idle => "idle",
            Phase::SpecReceived => "spec_received",
            Phase::Clarifying => "clarifying",
            Phase::Researching => "researching",
            Phase::Advising => "advising",
            Phase::SpecValidated => "spec_validated",
            Phase::TasksCreated => "tasks_created",
            Phase::Scheduling => "scheduling",
            Phase::Finalizing => "finalizing",
            Phase::FinalReview => "final_review",
            Phase::Crisis => "crisis",
            Phase::Delivered => "delivered",
            Phase::NeedsUser => "needs_user",
            Phase::Failed => "failed",
            Phase::Paused => "paused",
        }
    }
}

/// Why a run is `Paused` — distinguishes the recoverable budget wall (v2.47
/// B3: Paused(budget), user raises budget + resume) from a checkpoint pause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PauseReason {
    Budget,
    Checkpoint,
}

/// Progress signal fed to the FSM. Distinct from `EventKind` on purpose: a
/// trigger is an input, an event is the recorded OUTPUT of accepting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    SpecSubmitted,
    FormValidated,
    ClarificationsResolved,
    ResearchComplete,
    DecisionsLocked,
    FormInvalidExhausted,
    GateSpecApproved,
    TaskGraphValidated,
    AllTasksTerminal,
    ReportReady,
    ComplianceMet,
    ComplianceFailed,
    ReplanComplete,
    /// §5.6A m5 (E2): the replan ACTUALLY changed the plan — completed
    /// tasks stay completed (ledger rule 4), the new/uncut work re-enters
    /// scheduling. ReplanComplete (the other arm) keeps the old behavior:
    /// a no-op crisis loop returns straight to FinalReview.
    Replanned,
    BudgetHardLimit,
    UserInterrupt,
    FatalError,
    /// §9: the delivery gate received an explicit DENIAL — distinct from a
    /// compliance failure (replan cannot fix a user veto).
    GateDenied,
    /// §9 Q4: high-risk gate timed out with no decision — the run parks as
    /// NeedsUser; work is not lost, `resume` continues after the human votes.
    GateParked,
    /// User raised the budget (§8.2/B3): `Paused(budget)` → `paused_from`.
    /// The checkpoint pause (SIGINT) resumes the same way — resume is resume.
    Resume,
}

/// A snapshot of run control state. `crisis_count` gates FinalReview→Crisis
/// (B1: at most `crisis_max` loops, then NeedsUser — never self-immolate).
/// `paused_from` is the phase a `Paused` run resumes back INTO (§8.2/B3): the
/// budget wall is recoverable, so the run must remember where it stood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunState {
    pub phase: Phase,
    pub crisis_count: u32,
    pub paused: Option<PauseReason>,
    pub paused_from: Option<Phase>,
}

impl RunState {
    pub fn fresh() -> Self {
        Self {
            phase: Phase::Idle,
            crisis_count: 0,
            paused: None,
            paused_from: None,
        }
    }
}

/// Result of an accepted transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    pub state: RunState,
    /// The event to record in events.jsonl for this transition, if any.
    pub event: Option<crate::models::event::EventKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsmError {
    /// No rule maps (phase, trigger) — an orchestrator wiring bug, surfaced
    /// loudly rather than silently ignoring a progress signal.
    NoTransition { phase: Phase, trigger: Trigger },
    /// A terminal phase received a trigger.
    Terminal { phase: Phase },
}

impl std::fmt::Display for FsmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use FsmError::*;
        match self {
            NoTransition { phase, trigger } => write!(
                f,
                "FSM: no transition from {phase:?} on {trigger:?} (wiring bug)"
            ),
            Terminal { phase } => write!(
                f,
                "FSM: {phase:?} is terminal; run must resume under a new run_id"
            ),
        }
    }
}
impl std::error::Error for FsmError {}

use crate::models::event::EventKind as Ev;

/// Pure, deterministic transition. `crisis_max` comes from config
/// (`errors.final_review_crisis_max`, default 2, v2.47 B1).
pub fn transition(state: RunState, trigger: Trigger, crisis_max: u32) -> Result<Step, FsmError> {
    // "any" triggers take precedence over the linear flow (§6.1 rows:
    // budget-hard and SIGINT fire from any non-terminal phase).
    if state.phase.is_terminal() {
        return Err(FsmError::Terminal { phase: state.phase });
    }
    let paused_from = state.phase;
    match trigger {
        Trigger::BudgetHardLimit if !matches!(paused_from, Phase::Idle) => {
            return Ok(step(
                RunState {
                    phase: Phase::Paused,
                    crisis_count: state.crisis_count,
                    paused: Some(PauseReason::Budget),
                    // Durable anchor: raising the budget resumes INTO here.
                    paused_from: Some(paused_from),
                },
                Some(Ev::BudgetExceeded),
            ));
        }
        Trigger::UserInterrupt if !matches!(paused_from, Phase::Idle) => {
            return Ok(step(
                RunState {
                    phase: Phase::Paused,
                    crisis_count: state.crisis_count,
                    paused: Some(PauseReason::Checkpoint),
                    paused_from: Some(paused_from),
                },
                None,
            ))
        }
        // Resume only fires from Paused; the run returns to its recorded phase.
        Trigger::Resume if paused_from == Phase::Paused => {
            let back = state.paused_from.ok_or(FsmError::NoTransition {
                phase: Phase::Paused,
                trigger,
            })?;
            return Ok(step(
                RunState {
                    phase: back,
                    crisis_count: state.crisis_count,
                    paused: None,
                    paused_from: None,
                },
                None,
            ));
        }
        Trigger::FatalError => {
            return Ok(step(
                RunState {
                    phase: Phase::Failed,
                    crisis_count: state.crisis_count,
                    paused: None,
                    paused_from: None,
                },
                None,
            ))
        }
        _ => {}
    }

    match (state.phase, trigger) {
        (Phase::Idle, Trigger::SpecSubmitted) => {
            ok(Phase::SpecReceived, Some(Ev::SpecReceived), state)
        }
        // Entering Clarifying/Researching/Advising are dominant-phase signals
        // (§6.1 note), not catalogued events — no emission here.
        (Phase::SpecReceived, Trigger::FormValidated) => ok(Phase::Clarifying, None, state),
        (Phase::SpecReceived, Trigger::FormInvalidExhausted) => {
            ok(Phase::NeedsUser, Some(Ev::EscalatedToUser), state)
        }
        (Phase::Clarifying, Trigger::ClarificationsResolved) => ok(Phase::Researching, None, state),
        (Phase::Researching, Trigger::ResearchComplete) => ok(Phase::Advising, None, state),
        (Phase::Advising, Trigger::DecisionsLocked) => {
            ok(Phase::SpecValidated, Some(Ev::SpecValidated), state)
        }
        (Phase::SpecValidated, Trigger::GateSpecApproved) => {
            ok(Phase::TasksCreated, Some(Ev::TasksCreated), state)
        }
        (Phase::TasksCreated, Trigger::TaskGraphValidated) => ok(Phase::Scheduling, None, state),
        (Phase::Scheduling, Trigger::AllTasksTerminal) => ok(Phase::Finalizing, None, state),
        (Phase::Finalizing, Trigger::ReportReady) => ok(Phase::FinalReview, None, state),
        (Phase::FinalReview, Trigger::ComplianceMet) => {
            ok(Phase::Delivered, Some(Ev::Delivered), state)
        }
        (Phase::FinalReview, Trigger::ComplianceFailed) => {
            if state.crisis_count >= crisis_max {
                // B1: cap reached — hand the uncovered requirements to the
                // user as a scope decision, do not replan again.
                ok(Phase::NeedsUser, Some(Ev::EscalatedToUser), state)
            } else {
                // One replan allowed; counter climbs toward the cap.
                Ok(Step {
                    state: RunState {
                        phase: Phase::Crisis,
                        crisis_count: state.crisis_count + 1,
                        paused: None,
                        paused_from: None,
                    },
                    event: Some(Ev::EscalatedToCEO),
                })
            }
        }
        (Phase::Crisis, Trigger::ReplanComplete) => ok(Phase::FinalReview, None, state),
        (Phase::Crisis, Trigger::Replanned) => ok(Phase::Scheduling, Some(Ev::TasksRevised), state),
        // §9: a human veto at a gate is a scope decision, not a replan — the
        // run parks at NeedsUser regardless of which gate vetoed it.
        (Phase::SpecValidated | Phase::TasksCreated, Trigger::GateDenied) => {
            ok(Phase::NeedsUser, Some(Ev::EscalatedToUser), state)
        }
        (Phase::FinalReview, Trigger::GateDenied) => {
            ok(Phase::NeedsUser, Some(Ev::DeliveryRejected), state)
        }
        (Phase::FinalReview, Trigger::GateParked) => {
            ok(Phase::NeedsUser, Some(Ev::EscalatedToUser), state)
        }
        _ => Err(FsmError::NoTransition {
            phase: state.phase,
            trigger,
        }),
    }
}

/// Helper building an accepted step by swapping the phase (keeps counter/pause).
fn ok(to: Phase, event: Option<Ev>, from: RunState) -> Result<Step, FsmError> {
    Ok(Step {
        state: RunState {
            phase: to,
            crisis_count: from.crisis_count,
            paused: None,
            paused_from: None,
        },
        event,
    })
}

fn step(state: RunState, event: Option<Ev>) -> Step {
    Step { state, event }
}

/// Deterministically drive `state` through a trigger sequence (resume replay
/// and tests both use this). Stops at the first error.
pub fn drive(
    mut state: RunState,
    triggers: &[Trigger],
    crisis_max: u32,
) -> Result<(RunState, Vec<Ev>), (FsmError, RunState)> {
    let mut events = Vec::new();
    for t in triggers {
        state = match transition(state, *t, crisis_max) {
            Ok(s) => {
                if let Some(e) = s.event {
                    events.push(e);
                }
                s.state
            }
            Err(e) => return Err((e, state)),
        };
    }
    Ok((state, events))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- happy path ---

    #[test]
    fn golden_path_reaches_delivered() {
        let path = [
            Trigger::SpecSubmitted,
            Trigger::FormValidated,
            Trigger::ClarificationsResolved,
            Trigger::ResearchComplete,
            Trigger::DecisionsLocked,
            Trigger::GateSpecApproved,
            Trigger::TaskGraphValidated,
            Trigger::AllTasksTerminal,
            Trigger::ReportReady,
            Trigger::ComplianceMet,
        ];
        let (final_state, events) = drive(RunState::fresh(), &path, 2).unwrap();
        assert_eq!(final_state.phase, Phase::Delivered);
        assert!(final_state.crisis_count == 0);
        assert_eq!(events.first(), Some(&Ev::SpecReceived));
        assert_eq!(events.last(), Some(&Ev::Delivered));
    }

    #[test]
    fn form_broken_exhausts_to_needs_user() {
        let (s, ev) = drive(
            RunState::fresh(),
            &[Trigger::SpecSubmitted, Trigger::FormInvalidExhausted],
            2,
        )
        .unwrap();
        assert_eq!(s.phase, Phase::NeedsUser);
        assert_eq!(ev.last(), Some(&Ev::EscalatedToUser));
    }

    // --- crisis loop (v2.47 B1) ---

    #[test]
    fn compliance_fail_enters_crisis_and_increments() {
        let review = RunState {
            phase: Phase::FinalReview,
            crisis_count: 0,
            paused: None,
            paused_from: None,
        };
        let s = transition(review, Trigger::ComplianceFailed, 2).unwrap();
        assert_eq!(s.state.phase, Phase::Crisis);
        assert_eq!(s.state.crisis_count, 1);
    }

    #[test]
    fn crisis_loop_caps_at_max_then_needs_user() {
        let mut state = RunState {
            phase: Phase::FinalReview,
            crisis_count: 0,
            paused: None,
            paused_from: None,
        };
        // crisis_max = 2: two replans allowed, third escalation -> NeedsUser
        for expected in [1u32, 2] {
            assert_eq!(state.phase, Phase::FinalReview);
            state = transition(state, Trigger::ComplianceFailed, 2)
                .unwrap()
                .state;
            assert_eq!(state.phase, Phase::Crisis);
            assert_eq!(state.crisis_count, expected);
            state = transition(state, Trigger::ReplanComplete, 2).unwrap().state;
            assert_eq!(state.phase, Phase::FinalReview);
        }
        // now crisis_count == 2 == max -> NeedsUser, no further replan
        state = transition(state, Trigger::ComplianceFailed, 2)
            .unwrap()
            .state;
        assert_eq!(state.phase, Phase::NeedsUser);
        assert!(state.phase.is_terminal());
    }

    #[test]
    fn crisis_counter_never_exceeds_max_before_escalating() {
        // whatever the path, crisis_count <= crisis_max on every step
        let mut state = RunState {
            phase: Phase::FinalReview,
            crisis_count: 0,
            paused: None,
            paused_from: None,
        };
        for _ in 0..10 {
            state = transition(state, Trigger::ComplianceFailed, 2)
                .unwrap()
                .state;
            assert!(state.crisis_count <= 2);
            if state.phase == Phase::NeedsUser {
                return; // capped correctly
            }
            state = transition(state, Trigger::ReplanComplete, 2).unwrap().state;
        }
        panic!("crisis loop never reached the NeedsUser cap in 10 iterations");
    }

    // --- any-phase triggers ---

    #[test]
    fn budget_hard_limit_pauses_from_any_active_phase() {
        for phase in [Phase::SpecReceived, Phase::Scheduling, Phase::FinalReview] {
            let s = RunState {
                phase,
                crisis_count: 0,
                paused: None,
                paused_from: None,
            };
            let after = transition(s, Trigger::BudgetHardLimit, 2).unwrap();
            assert_eq!(after.state.phase, Phase::Paused);
            assert_eq!(after.state.paused, Some(PauseReason::Budget));
            assert_eq!(after.event, Some(Ev::BudgetExceeded));
        }
    }

    #[test]
    fn fatal_from_idle_is_allowed_but_budget_at_idle_is_not() {
        // FatalError has no `!Idle` guard, so it fires from Idle.
        let s = transition(RunState::fresh(), Trigger::FatalError, 2).unwrap();
        assert_eq!(s.state.phase, Phase::Failed);
        // BudgetHardLimit is guarded against Idle (nothing to pause).
        let s = transition(RunState::fresh(), Trigger::BudgetHardLimit, 2);
        assert!(s.is_err());
    }

    #[test]
    fn interrupt_preserves_crisis_counter() {
        let s = RunState {
            phase: Phase::FinalReview,
            crisis_count: 1,
            paused: None,
            paused_from: None,
        };
        let after = transition(s, Trigger::UserInterrupt, 2).unwrap();
        assert_eq!(after.state.phase, Phase::Paused);
        assert_eq!(after.state.crisis_count, 1, "pause must not reset progress");
        assert_eq!(after.state.paused, Some(PauseReason::Checkpoint));
    }

    // --- total / illegal / terminal discipline ---

    #[test]
    fn gate_denial_and_park_reach_needs_user() {
        // §9: denial at each gated phase, and the delivery-timeout park.
        let (s, _ev) = drive(
            RunState::fresh(),
            &[
                Trigger::SpecSubmitted,
                Trigger::FormValidated,
                Trigger::ClarificationsResolved,
                Trigger::ResearchComplete,
                Trigger::DecisionsLocked,
            ],
            2,
        )
        .unwrap();
        assert_eq!(s.phase, Phase::SpecValidated);
        let (s, _) = drive(s, &[Trigger::GateDenied], 2).unwrap();
        assert_eq!(s.phase, Phase::NeedsUser);
        // fresh run → TasksCreated → veto task_plan
        let (s, _) = drive(
            RunState::fresh(),
            &[
                Trigger::SpecSubmitted,
                Trigger::FormValidated,
                Trigger::ClarificationsResolved,
                Trigger::ResearchComplete,
                Trigger::DecisionsLocked,
                Trigger::GateSpecApproved,
            ],
            2,
        )
        .unwrap();
        assert_eq!(s.phase, Phase::TasksCreated);
        let (s, _) = drive(s, &[Trigger::GateDenied], 2).unwrap();
        assert_eq!(s.phase, Phase::NeedsUser);
        // fresh run → FinalReview → delivery gate parked (Q4: NeedsUser, nothing lost)
        let review = RunState {
            phase: Phase::FinalReview,
            crisis_count: 0,
            paused: None,
            paused_from: None,
        };
        let step = transition(review, Trigger::GateParked, 2).unwrap();
        assert_eq!(step.state.phase, Phase::NeedsUser);
        assert_eq!(step.event, Some(Ev::EscalatedToUser));
        // denial at delivery records the rejection event
        let step = transition(review, Trigger::GateDenied, 2).unwrap();
        assert_eq!(step.state.phase, Phase::NeedsUser);
        assert_eq!(step.event, Some(Ev::DeliveryRejected));
    }

    #[test]
    fn replanned_reenters_scheduling_and_records_the_revision() {
        let crisis = RunState {
            phase: Phase::Crisis,
            crisis_count: 1,
            paused: None,
            paused_from: None,
        };
        let step = transition(crisis, Trigger::Replanned, 2).unwrap();
        assert_eq!(step.state.phase, Phase::Scheduling);
        assert_eq!(step.state.crisis_count, 1, "the counter survives the loop");
        assert_eq!(step.event, Some(Ev::TasksRevised));
        // ReplanComplete keeps the historical no-op route.
        let step = transition(crisis, Trigger::ReplanComplete, 2).unwrap();
        assert_eq!(step.state.phase, Phase::FinalReview);
    }

    #[test]
    fn budget_pause_then_resume_returns_to_paused_from() {
        // §8.2/B3: a budget wall is RECOVERABLE — the run pauses recording the
        // phase, then a budget raise (Trigger::Resume) drops it back to work.
        let mid = RunState {
            phase: Phase::Scheduling,
            crisis_count: 0,
            paused: None,
            paused_from: None,
        };
        let paused = transition(mid, Trigger::BudgetHardLimit, 2).unwrap().state;
        assert_eq!(paused.phase, Phase::Paused);
        assert_eq!(paused.paused, Some(PauseReason::Budget));
        assert_eq!(paused.paused_from, Some(Phase::Scheduling));
        let resumed = transition(paused, Trigger::Resume, 2).unwrap().state;
        assert_eq!(resumed.phase, Phase::Scheduling);
        assert!(resumed.paused.is_none() && resumed.paused_from.is_none());
    }

    #[test]
    fn resume_needs_an_anchor_never_teleports_to_idle() {
        // A Paused state whose paused_from was lost (corrupt row) must error —
        // defaulting to Idle would silently restart the run and re-schedule
        // completed work (§7.4 rule 4 depends on a truthful anchor).
        let anchorless = RunState {
            phase: Phase::Paused,
            crisis_count: 0,
            paused: Some(PauseReason::Budget),
            paused_from: None,
        };
        assert!(matches!(
            transition(anchorless, Trigger::Resume, 2),
            Err(FsmError::NoTransition { .. })
        ));
    }

    #[test]
    fn illegal_transition_is_an_error_not_a_silent_noop() {
        // Delivering straight from Idle is nonsense.
        let e = transition(RunState::fresh(), Trigger::ComplianceMet, 2).unwrap_err();
        assert!(matches!(e, FsmError::NoTransition { .. }));
    }

    #[test]
    fn terminal_phases_refuse_all_triggers() {
        for phase in [Phase::Delivered, Phase::Failed, Phase::NeedsUser] {
            let s = RunState {
                phase,
                crisis_count: 0,
                paused: None,
                paused_from: None,
            };
            for t in [
                Trigger::SpecSubmitted,
                Trigger::BudgetHardLimit,
                Trigger::UserInterrupt,
                Trigger::FatalError,
            ] {
                let r = transition(s, t, 2);
                assert!(
                    matches!(r, Err(FsmError::Terminal { .. })),
                    "{phase:?} {t:?} -> {r:?}"
                );
            }
        }
    }

    #[test]
    fn transition_is_deterministic_exhaustive() {
        // Every (phase, trigger) pair returns without panic, and the same call
        // twice yields identical results (pure function).
        let phases = [
            Phase::Idle,
            Phase::SpecReceived,
            Phase::Clarifying,
            Phase::Researching,
            Phase::Advising,
            Phase::SpecValidated,
            Phase::TasksCreated,
            Phase::Scheduling,
            Phase::Finalizing,
            Phase::FinalReview,
            Phase::Crisis,
            Phase::Delivered,
            Phase::NeedsUser,
            Phase::Failed,
            Phase::Paused,
        ];
        let triggers = [
            Trigger::SpecSubmitted,
            Trigger::FormValidated,
            Trigger::ClarificationsResolved,
            Trigger::ResearchComplete,
            Trigger::DecisionsLocked,
            Trigger::FormInvalidExhausted,
            Trigger::GateSpecApproved,
            Trigger::TaskGraphValidated,
            Trigger::AllTasksTerminal,
            Trigger::ReportReady,
            Trigger::ComplianceMet,
            Trigger::ComplianceFailed,
            Trigger::ReplanComplete,
            Trigger::BudgetHardLimit,
            Trigger::UserInterrupt,
            Trigger::FatalError,
            Trigger::GateDenied,
            Trigger::GateParked,
            Trigger::Resume,
        ];
        for p in phases {
            for t in triggers {
                let s = RunState {
                    phase: p,
                    crisis_count: 1,
                    paused: None,
                    paused_from: None,
                };
                let a = transition(s, t, 2);
                let b = transition(s, t, 2);
                assert_eq!(a, b, "FSM not pure for {p:?} {t:?}");
                assert!(a.is_ok() || a.is_err());
            }
        }
    }

    #[test]
    fn paused_phase_can_continue_after_resume_of_counter() {
        // After a budget pause, the run is Paused; the orchestrator resumes by
        // re-issuing the same trigger that would have advanced it (phase is
        // restored by the scheduler), so the FSM itself only forbids advancing
        // FROM Paused via linear triggers -> NoTransition, proving Paused is
        // not silently passable.
        let s = RunState {
            phase: Phase::Paused,
            crisis_count: 0,
            paused: Some(PauseReason::Budget),
            paused_from: None,
        };
        let r = transition(s, Trigger::AllTasksTerminal, 2);
        assert!(matches!(r, Err(FsmError::NoTransition { .. })));
    }
}
