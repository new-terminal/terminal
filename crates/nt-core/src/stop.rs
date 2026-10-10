//! The stop steps for one agent, as a state machine. The actor takes each
//! returned [`Step`] and checks [`StopSteps::deadline`] with its receive
//! time limit, so no timer thread exists.
//!
//! 1. The actor denies the agent's waiting permissions (not tracked here).
//! 2. An agent in a turn with a live stdin writer gets the interrupt, and up
//!    to 3 s for its `result`.
//! 3. SIGTERM to the process group. The CLI then ends the commands it
//!    started in the background.
//! 4. SIGKILL to the group if the process is still alive 2 s later.

use std::time::{Duration, Instant};

const RESULT_WAIT: Duration = Duration::from_secs(3);
const TERM_WAIT: Duration = Duration::from_secs(2);
/// How long an agent that closed its stdout may take to exit by itself.
const CLOSED_OUTPUT_WAIT: Duration = Duration::from_secs(2);

/// What the actor does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    SendInterrupt,
    Terminate,
    Kill,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    AwaitingResult { until: Instant },
    AwaitingExit { until: Instant },
    AwaitingExitAfterClose { until: Instant },
    Killed,
}

/// The strongest signal sent so far, as `app.log` records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignalSent {
    None,
    Term,
    Kill,
}

impl SignalSent {
    pub const fn field(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Term => "term",
            Self::Kill => "kill",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StopSteps {
    phase: Phase,
    signal: SignalSent,
    /// The agent closed its stdout on its own, so its end shows as failed.
    closed_output: bool,
}

impl StopSteps {
    /// Starts the steps at step 2. `can_interrupt` is true for an agent in
    /// a turn whose stdin writer is alive.
    pub fn begin(now: Instant, can_interrupt: bool) -> (Self, Step) {
        if can_interrupt {
            let steps = Self {
                phase: Phase::AwaitingResult {
                    until: now + RESULT_WAIT,
                },
                signal: SignalSent::None,
                closed_output: false,
            };
            (steps, Step::SendInterrupt)
        } else {
            (Self::terminating(now, false), Step::Terminate)
        }
    }

    /// Starts the steps for an agent that closed stdout but did not exit:
    /// it gets a short wait before SIGTERM.
    pub fn after_closed_output(now: Instant) -> Self {
        Self {
            phase: Phase::AwaitingExitAfterClose {
                until: now + CLOSED_OUTPUT_WAIT,
            },
            signal: SignalSent::None,
            closed_output: true,
        }
    }

    /// The turn's `result` arrived.
    pub fn on_result(&mut self, now: Instant) -> Option<Step> {
        match self.phase {
            Phase::AwaitingResult { .. } => {
                *self = Self::terminating(now, self.closed_output);
                Some(Step::Terminate)
            }
            _ => None,
        }
    }

    /// The step due at `now`, if the current wait has passed.
    pub fn on_deadline(&mut self, now: Instant) -> Option<Step> {
        if self.deadline().is_none_or(|until| now < until) {
            return None;
        }
        match self.phase {
            Phase::AwaitingResult { .. } | Phase::AwaitingExitAfterClose { .. } => {
                *self = Self::terminating(now, self.closed_output);
                Some(Step::Terminate)
            }
            Phase::AwaitingExit { .. } => {
                self.phase = Phase::Killed;
                self.signal = SignalSent::Kill;
                Some(Step::Kill)
            }
            Phase::Killed => None,
        }
    }

    pub const fn deadline(&self) -> Option<Instant> {
        match self.phase {
            Phase::AwaitingResult { until }
            | Phase::AwaitingExit { until }
            | Phase::AwaitingExitAfterClose { until } => Some(until),
            Phase::Killed => None,
        }
    }

    pub const fn signal(&self) -> SignalSent {
        self.signal
    }

    pub const fn closed_output(&self) -> bool {
        self.closed_output
    }

    fn terminating(now: Instant, closed_output: bool) -> Self {
        Self {
            phase: Phase::AwaitingExit {
                until: now + TERM_WAIT,
            },
            signal: SignalSent::Term,
            closed_output,
        }
    }
}
