//! Security-relevant events, logged with target `audit` and a fixed `event`
//! name so they can be filtered out of the rest of the log. Inside a request
//! they carry the request span: `trace_id`, `client`, `user`, method and
//! path.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditEvent {
    LoginFailed,
    Forbidden,
    PublisherKicked,
    KickFailed,
    MediaAccessDenied,
}

impl AuditEvent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LoginFailed => "login_failed",
            Self::Forbidden => "forbidden",
            Self::PublisherKicked => "publisher_kicked",
            Self::KickFailed => "kick_failed",
            Self::MediaAccessDenied => "media_access_denied",
        }
    }
}

/// `audit!(warn, AuditEvent::Forbidden, code, "message")` logs at `warn`
/// with target `audit` and `event = "forbidden"`, then the remaining fields.
macro_rules! audit {
    ($level:ident, $event:expr, $($rest:tt)+) => {
        ::tracing::$level!(
            target: "audit",
            event = $crate::audit::AuditEvent::as_str($event),
            $($rest)+
        )
    };
}

pub(crate) use audit;
