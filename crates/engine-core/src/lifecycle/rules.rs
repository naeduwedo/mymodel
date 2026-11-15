//! Core lifecycle state machine (spec §3.3).
//!
//! ```text
//! Stopped -> Starting -> Ready -> Degraded -> ShuttingDown -> Stopped
//! ```
//!
//! Transitions are validated here rather than scattered across modules so a
//! UI can never observe an impossible sequence.

use gguf_engine_event_bus::CoreState;
use thiserror::Error;

/// Errors raised when a transition is requested
/// (see [`crate::Lifecycle::transition`] and [`validate`]).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TransitionError {
    /// The requested transition is not part of the state machine.
    #[error("illegal core transition: {} -> {}", from.as_str(), to.as_str())]
    Illegal {
        /// Current state.
        from: CoreState,
        /// Requested state.
        to: CoreState,
    },
    /// Another thread is currently transitioning.
    #[error("another lifecycle transition is in progress")]
    Busy,
}

/// Allowed successor states for a state.
pub fn allowed_successors(state: CoreState) -> &'static [CoreState] {
    match state {
        CoreState::Stopped => &[CoreState::Starting],
        CoreState::Starting => &[
            CoreState::Ready,
            CoreState::Degraded,
            CoreState::ShuttingDown,
        ],
        CoreState::Ready => &[CoreState::Degraded, CoreState::ShuttingDown],
        CoreState::Degraded => &[CoreState::Ready, CoreState::ShuttingDown],
        CoreState::ShuttingDown => &[CoreState::Stopped],
    }
}

/// Returns true when `from -> to` is a legal transition.
pub fn can_transition(from: CoreState, to: CoreState) -> bool {
    allowed_successors(from).contains(&to)
}

/// Validate a transition, returning the error the caller should surface.
pub fn validate(from: CoreState, to: CoreState) -> Result<(), TransitionError> {
    if can_transition(from, to) {
        Ok(())
    } else {
        Err(TransitionError::Illegal { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [CoreState; 5] = [
        CoreState::Stopped,
        CoreState::Starting,
        CoreState::Ready,
        CoreState::Degraded,
        CoreState::ShuttingDown,
    ];

    #[test]
    fn happy_path_is_allowed() {
        let path = [
            CoreState::Stopped,
            CoreState::Starting,
            CoreState::Ready,
            CoreState::Degraded,
            CoreState::Ready,
            CoreState::ShuttingDown,
            CoreState::Stopped,
        ];
        for pair in path.windows(2) {
            validate(pair[0], pair[1]).unwrap_or_else(|e| panic!("{e}"));
        }
    }

    #[test]
    fn self_transition_is_rejected() {
        for state in ALL {
            assert!(matches!(
                validate(state, state),
                Err(TransitionError::Illegal { .. })
            ));
        }
    }

    #[test]
    fn cannot_jump_from_stopped_to_ready() {
        let err = validate(CoreState::Stopped, CoreState::Ready).unwrap_err();
        assert_eq!(
            err,
            TransitionError::Illegal {
                from: CoreState::Stopped,
                to: CoreState::Ready
            }
        );
        assert_eq!(err.to_string(), "illegal core transition: stopped -> ready");
    }

    #[test]
    fn cannot_leave_shutting_down_except_to_stopped() {
        for state in ALL {
            let allowed = state == CoreState::Stopped;
            assert_eq!(can_transition(CoreState::ShuttingDown, state), allowed);
        }
    }

    #[test]
    fn successors_are_symmetric_with_can_transition() {
        for from in ALL {
            for to in ALL {
                assert_eq!(
                    can_transition(from, to),
                    allowed_successors(from).contains(&to),
                    "{from:?} -> {to:?}"
                );
            }
        }
    }
}
