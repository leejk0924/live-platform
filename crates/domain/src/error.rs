/// How a domain failure is classified for callers; adapters map it to their
/// own protocol (HTTP status, gRPC code, ...).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidInput,
    Unauthenticated,
    Forbidden,
    NotFound,
    Conflict,
}

/// Implemented by every feature's error enum. `Display` is the user-facing
/// message, so it must not carry internal detail.
pub trait DomainError: std::error::Error + Send + Sync + 'static {
    fn kind(&self) -> ErrorKind;

    /// Stable SCREAMING_SNAKE identifier clients can match on.
    fn code(&self) -> &'static str;
}
