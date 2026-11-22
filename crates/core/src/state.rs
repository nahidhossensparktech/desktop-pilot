//! Agent states (ARCHITECTURE §4). Transitions are added in M3.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentState {
    Idle,
    Thinking,
    AwaitingConfirmation,
    Acting,
    Paused,
    Stopped,
    Completed,
    Error,
}

impl AgentState {
    /// True while a task is in progress, i.e. when FORCE STOP must be enabled.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Thinking | Self::AwaitingConfirmation | Self::Acting | Self::Paused
        )
    }
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Idle => "Idle",
            Self::Thinking => "Thinking",
            Self::AwaitingConfirmation => "Waiting for approval",
            Self::Acting => "Acting",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
            Self::Completed => "Completed",
            Self::Error => "Error",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::AgentState::*;

    #[test]
    fn stop_is_enabled_exactly_in_active_states() {
        let active = [Thinking, AwaitingConfirmation, Acting, Paused];
        let inactive = [Idle, Stopped, Completed, Error];
        assert!(active.iter().all(|s| s.is_active()));
        assert!(inactive.iter().all(|s| !s.is_active()));
    }
}
