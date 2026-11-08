//! Comments on a broadcast: what a viewer may write, how often, and how a
//! client follows every change to them.

use std::time::{Duration, SystemTime};

use thiserror::Error;

use crate::{
    auth::UserId,
    broadcast::{BroadcastId, BroadcastStatus, Version},
    error::{DomainError, ErrorKind},
    id::SnowflakeId,
};

pub type CommentId = SnowflakeId;

pub const COMMENT_MAX_CHARS: usize = 500;
/// Comments per list page when the client names no size, and the most it
/// may ask for.
pub const PAGE_SIZE_DEFAULT: u32 = 50;
pub const PAGE_SIZE_MAX: u32 = 100;

/// Every user may post at most once per interval, across all broadcasts.
pub const POST_INTERVAL: Duration = Duration::from_secs(1);
/// The longest slow mode a broadcaster may set.
pub const SLOW_MODE_MAX_SECS: u64 = 300;
pub const SLOW_MODE_MAX: Duration = Duration::from_secs(SLOW_MODE_MAX_SECS);

/// Whether a broadcast in `status` takes new comments. An ended broadcast
/// keeps its comments but takes no more.
pub fn accepts_comments(status: BroadcastStatus) -> bool {
    status != BroadcastStatus::Ended
}

/// Trimmed, 1 to 500 characters, on one line: control characters, line
/// breaks included, are refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommentBody(String);

impl CommentBody {
    pub fn parse(raw: &str) -> Result<Self, CommentError> {
        let trimmed = raw.trim();
        let valid_len = (1..=COMMENT_MAX_CHARS).contains(&trimmed.chars().count());
        if valid_len && !trimmed.chars().any(char::is_control) {
            Ok(Self(trimmed.to_owned()))
        } else {
            Err(CommentError::InvalidBody)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where a change to a comment sits in its broadcast's change feed. Every
/// write to a comment (post, edit, delete) gets a higher one than any write
/// before it, so "the changes after R" is everything a client holding R has
/// not seen. [`Revision::START`] comes before every change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(i64);

impl Revision {
    pub const START: Self = Self(0);

    /// A position in the feed, [`Revision::START`] included: what a client
    /// follows from.
    pub fn new(raw: i64) -> Option<Self> {
        (raw >= 0).then_some(Self(raw))
    }

    /// The revision of a change, which always comes after
    /// [`Revision::START`].
    pub fn of_change(raw: i64) -> Option<Self> {
        (raw >= 1).then_some(Self(raw))
    }

    /// Decimal digits only, as the server sent it.
    pub fn parse(raw: &str) -> Option<Self> {
        if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        raw.parse().ok().and_then(Self::new)
    }

    pub fn get(self) -> i64 {
        self.0
    }
}

/// How often viewers may post to one broadcast, set by its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChatPolicy {
    slow_mode: Duration,
}

impl ChatPolicy {
    pub const OFF: Self = Self {
        slow_mode: Duration::ZERO,
    };

    /// 0 turns slow mode off; at most [`SLOW_MODE_MAX`] seconds.
    pub fn from_secs(secs: u32) -> Result<Self, CommentError> {
        let slow_mode = Duration::from_secs(u64::from(secs));
        if slow_mode <= SLOW_MODE_MAX {
            Ok(Self { slow_mode })
        } else {
            Err(CommentError::InvalidSlowMode)
        }
    }

    pub fn slow_mode_secs(self) -> u32 {
        u32::try_from(self.slow_mode.as_secs()).expect("at most SLOW_MODE_MAX")
    }

    /// How long one viewer must wait between posts to this broadcast, beyond
    /// [`POST_INTERVAL`]. The owner is never slowed down in their own chat.
    pub fn slow_mode_for(self, is_owner: bool) -> Option<Duration> {
        (!is_owner && !self.slow_mode.is_zero()).then_some(self.slow_mode)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub id: CommentId,
    pub broadcast_id: BroadcastId,
    pub author_id: UserId,
    /// `None` once deleted: the comment stays in the change feed as a
    /// tombstone so followers can remove it.
    pub body: Option<CommentBody>,
    pub version: Version,
    pub created_at: SystemTime,
    pub edited: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum CommentError {
    #[error("comment must be 1 to {COMMENT_MAX_CHARS} characters on one line")]
    InvalidBody,
    #[error("slow mode must be between 0 and {SLOW_MODE_MAX_SECS} seconds")]
    InvalidSlowMode,
    #[error("page size must be between 1 and {PAGE_SIZE_MAX}")]
    InvalidPageSize,
    #[error("broadcast not found")]
    BroadcastNotFound,
    #[error("comment not found")]
    NotFound,
    #[error("only the author can change this comment")]
    NotAuthor,
    #[error("only the owner can change this broadcast")]
    NotBroadcastOwner,
    #[error("comment was changed by another request; reload and try again")]
    VersionConflict,
    #[error("the broadcast has ended and takes no more comments")]
    Closed,
}

impl DomainError for CommentError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidBody | Self::InvalidSlowMode | Self::InvalidPageSize => {
                ErrorKind::InvalidInput
            }
            Self::BroadcastNotFound | Self::NotFound => ErrorKind::NotFound,
            Self::NotAuthor | Self::NotBroadcastOwner => ErrorKind::Forbidden,
            Self::VersionConflict | Self::Closed => ErrorKind::Conflict,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::InvalidBody => "INVALID_COMMENT",
            Self::InvalidSlowMode => "INVALID_SLOW_MODE",
            Self::InvalidPageSize => "INVALID_PAGE_SIZE",
            Self::BroadcastNotFound => "BROADCAST_NOT_FOUND",
            Self::NotFound => "COMMENT_NOT_FOUND",
            Self::NotAuthor => "NOT_COMMENT_AUTHOR",
            Self::NotBroadcastOwner => "NOT_BROADCAST_OWNER",
            Self::VersionConflict => "VERSION_CONFLICT",
            Self::Closed => "COMMENTS_CLOSED",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_is_trimmed_and_counts_characters() {
        assert_eq!(CommentBody::parse("  안녕 ").unwrap().as_str(), "안녕");
        assert!(CommentBody::parse(&"가".repeat(500)).is_ok(), "1500 bytes");
        assert!(CommentBody::parse(&"😀".repeat(500)).is_ok(), "2000 bytes");
        for bad in [String::new(), "  \n ".into(), "a".repeat(501)] {
            assert_eq!(
                CommentBody::parse(&bad),
                Err(CommentError::InvalidBody),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn body_is_one_line_without_control_characters() {
        for bad in ["a\nb", "a\rb", "a\tb", "a\u{0}b", "a\u{7f}b", "a\u{85}b"] {
            assert_eq!(
                CommentBody::parse(bad),
                Err(CommentError::InvalidBody),
                "{bad:?}"
            );
        }
        assert!(CommentBody::parse("<script>\"'\\").is_ok());
    }

    #[test]
    fn revision_is_a_non_negative_decimal() {
        assert_eq!(Revision::parse("42").map(Revision::get), Some(42));
        assert_eq!(Revision::parse("0"), Some(Revision::START));
        for bad in ["", "-1", "+1", " 1", "1.0", "x", "99999999999999999999"] {
            assert_eq!(Revision::parse(bad), None, "{bad:?}");
        }
        assert!(Some(Revision::START) < Revision::new(1));
        assert!(Revision::new(1) < Revision::new(2));
    }

    #[test]
    fn a_change_comes_after_the_start() {
        assert_eq!(Revision::of_change(1), Revision::new(1));
        for bad in [0, -1] {
            assert_eq!(Revision::of_change(bad), None, "{bad}");
        }
    }

    #[test]
    fn slow_mode_is_0_to_300_seconds() {
        assert_eq!(ChatPolicy::from_secs(0), Ok(ChatPolicy::OFF));
        assert_eq!(ChatPolicy::from_secs(300).unwrap().slow_mode_secs(), 300);
        assert_eq!(
            ChatPolicy::from_secs(301),
            Err(CommentError::InvalidSlowMode)
        );
        assert_eq!(
            CommentError::InvalidSlowMode.to_string(),
            "slow mode must be between 0 and 300 seconds"
        );
    }

    #[test]
    fn slow_mode_applies_to_viewers_only() {
        let slow = ChatPolicy::from_secs(30).unwrap();
        assert_eq!(slow.slow_mode_for(false), Some(Duration::from_secs(30)));
        assert_eq!(slow.slow_mode_for(true), None);
        assert_eq!(ChatPolicy::OFF.slow_mode_for(false), None);
    }

    #[test]
    fn only_ended_broadcasts_refuse_comments() {
        let open: Vec<_> = BroadcastStatus::ALL
            .into_iter()
            .filter(|s| accepts_comments(*s))
            .collect();
        assert_eq!(
            open,
            [
                BroadcastStatus::Ready,
                BroadcastStatus::Live,
                BroadcastStatus::Interrupted
            ]
        );
    }

    #[test]
    fn errors_have_stable_codes_and_kinds() {
        for (err, kind, code) in [
            (
                CommentError::InvalidBody,
                ErrorKind::InvalidInput,
                "INVALID_COMMENT",
            ),
            (
                CommentError::InvalidSlowMode,
                ErrorKind::InvalidInput,
                "INVALID_SLOW_MODE",
            ),
            (
                CommentError::BroadcastNotFound,
                ErrorKind::NotFound,
                "BROADCAST_NOT_FOUND",
            ),
            (
                CommentError::NotFound,
                ErrorKind::NotFound,
                "COMMENT_NOT_FOUND",
            ),
            (
                CommentError::NotAuthor,
                ErrorKind::Forbidden,
                "NOT_COMMENT_AUTHOR",
            ),
            (
                CommentError::NotBroadcastOwner,
                ErrorKind::Forbidden,
                "NOT_BROADCAST_OWNER",
            ),
            (
                CommentError::VersionConflict,
                ErrorKind::Conflict,
                "VERSION_CONFLICT",
            ),
            (CommentError::Closed, ErrorKind::Conflict, "COMMENTS_CLOSED"),
        ] {
            assert_eq!((err.kind(), err.code()), (kind, code), "{err:?}");
        }
    }
}
