//! Session state machine (enum + match, no external crate).

/// Console session states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Connected,
    Authenticating,
    Authenticated,
    GrpcMode,
    CommandLoop,
    Quitting,
}
