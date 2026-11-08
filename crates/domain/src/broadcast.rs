//! Broadcasts: what a streamer registers before going live, and its lifecycle
//! status.

use std::time::{Duration, SystemTime};

use thiserror::Error;

use crate::{
    auth::UserId,
    error::{DomainError, ErrorKind},
    id::SnowflakeId,
};

pub type BroadcastId = SnowflakeId;

pub const TITLE_MAX_CHARS: usize = 100;
pub const DESCRIPTION_MAX_CHARS: usize = 2000;
/// Broadcasts per list page when the client names no size, and the most it
/// may ask for.
pub const PAGE_SIZE_DEFAULT: u32 = 20;
pub const PAGE_SIZE_MAX: u32 = 50;

/// How long an `INTERRUPTED` broadcast waits for its publisher to come back
/// before it ends.
pub const INTERRUPT_GRACE: Duration = Duration::from_secs(30);

/// `READY -> LIVE <-> INTERRUPTED -> ENDED`. Which status follows which is
/// decided only by [`BroadcastEvent::transition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BroadcastStatus {
    Ready,
    Live,
    Interrupted,
    Ended,
}

impl BroadcastStatus {
    pub const ALL: [Self; 4] = [Self::Ready, Self::Live, Self::Interrupted, Self::Ended];

    /// The only statuses a broadcast may be deleted in. Adapters bind this
    /// set into their conditional writes instead of restating it.
    pub const DELETABLE: [Self; 2] = [Self::Ready, Self::Ended];

    /// On air: the publisher may send media, the owner may stop it, and an
    /// owner has at most one broadcast in these statuses.
    pub const ACTIVE: [Self; 2] = [Self::Live, Self::Interrupted];

    pub fn is_deletable(self) -> bool {
        Self::DELETABLE.contains(&self)
    }

    pub fn is_active(self) -> bool {
        Self::ACTIVE.contains(&self)
    }

    /// The status `event` moves this one to, or `None` when the event does
    /// not apply here.
    pub fn apply(self, event: BroadcastEvent) -> Option<Self> {
        let transition = event.transition();
        transition.from.contains(&self).then_some(transition.to)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "READY",
            Self::Live => "LIVE",
            Self::Interrupted => "INTERRUPTED",
            Self::Ended => "ENDED",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.as_str() == raw)
    }
}

/// Something that changes a broadcast's status. The owner starts and stops
/// it, the media server's reports interrupt and resume it, and
/// [`INTERRUPT_GRACE`] running out expires it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BroadcastEvent {
    Start,
    Interrupt,
    Resume,
    Stop,
    Expire,
}

/// The statuses an event may leave and the one it enters. Adapters bind both
/// into one conditional write instead of restating them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub from: &'static [BroadcastStatus],
    pub to: BroadcastStatus,
}

impl BroadcastEvent {
    pub const ALL: [Self; 5] = [
        Self::Start,
        Self::Interrupt,
        Self::Resume,
        Self::Stop,
        Self::Expire,
    ];

    pub const fn transition(self) -> Transition {
        use BroadcastStatus::{Ended, Interrupted, Live, Ready};
        match self {
            Self::Start => Transition {
                from: &[Ready],
                to: Live,
            },
            Self::Interrupt => Transition {
                from: &[Live],
                to: Interrupted,
            },
            Self::Resume => Transition {
                from: &[Interrupted],
                to: Live,
            },
            Self::Stop => Transition {
                from: &BroadcastStatus::ACTIVE,
                to: Ended,
            },
            Self::Expire => Transition {
                from: &[Interrupted],
                to: Ended,
            },
        }
    }
}

/// Trimmed, 1 to 100 characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastTitle(String);

impl BroadcastTitle {
    pub fn parse(raw: &str) -> Result<Self, BroadcastError> {
        let trimmed = raw.trim();
        if (1..=TITLE_MAX_CHARS).contains(&trimmed.chars().count()) {
            Ok(Self(trimmed.to_owned()))
        } else {
            Err(BroadcastError::InvalidTitle)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// At most 2000 characters, kept as written. Empty is allowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastDescription(String);

impl BroadcastDescription {
    pub fn parse(raw: &str) -> Result<Self, BroadcastError> {
        if raw.chars().count() <= DESCRIPTION_MAX_CHARS {
            Ok(Self(raw.to_owned()))
        } else {
            Err(BroadcastError::InvalidDescription)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Optimistic-lock version of a broadcast; starts at 1 and grows by one per
/// write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(i32);

impl Version {
    pub const FIRST: Self = Self(1);

    pub fn new(raw: i32) -> Option<Self> {
        (raw >= 1).then_some(Self(raw))
    }

    pub fn get(self) -> i32 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broadcast {
    pub id: BroadcastId,
    pub owner_id: UserId,
    pub title: BroadcastTitle,
    pub description: BroadcastDescription,
    pub status: BroadcastStatus,
    pub version: Version,
    pub created_at: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BroadcastError {
    #[error("title must be 1 to {TITLE_MAX_CHARS} characters")]
    InvalidTitle,
    #[error("description must be at most {DESCRIPTION_MAX_CHARS} characters")]
    InvalidDescription,
    #[error("page size must be between 1 and {PAGE_SIZE_MAX}")]
    InvalidPageSize,
    #[error("broadcast not found")]
    NotFound,
    #[error("only the owner can change this broadcast")]
    NotOwner,
    #[error("broadcast was changed by another request; reload and try again")]
    VersionConflict,
    #[error("a broadcast that is live or interrupted cannot be deleted")]
    NotDeletable,
    #[error("an ended broadcast cannot go live again")]
    NotStartable,
    #[error("the broadcast is not live")]
    NotLive,
    #[error("the owner already has another live broadcast; end it first")]
    OwnerAlreadyLive,
}

impl DomainError for BroadcastError {
    fn kind(&self) -> ErrorKind {
        match self {
            Self::InvalidTitle | Self::InvalidDescription | Self::InvalidPageSize => {
                ErrorKind::InvalidInput
            }
            Self::NotFound => ErrorKind::NotFound,
            Self::NotOwner => ErrorKind::Forbidden,
            Self::VersionConflict
            | Self::NotDeletable
            | Self::NotStartable
            | Self::NotLive
            | Self::OwnerAlreadyLive => ErrorKind::Conflict,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::InvalidTitle => "INVALID_TITLE",
            Self::InvalidDescription => "INVALID_DESCRIPTION",
            Self::InvalidPageSize => "INVALID_PAGE_SIZE",
            Self::NotFound => "BROADCAST_NOT_FOUND",
            Self::NotOwner => "NOT_BROADCAST_OWNER",
            Self::VersionConflict => "VERSION_CONFLICT",
            Self::NotDeletable => "BROADCAST_NOT_DELETABLE",
            Self::NotStartable => "BROADCAST_NOT_STARTABLE",
            Self::NotLive => "BROADCAST_NOT_LIVE",
            Self::OwnerAlreadyLive => "OWNER_ALREADY_LIVE",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_is_trimmed_and_counts_characters() {
        assert_eq!(
            BroadcastTitle::parse("  첫 방송 \n").unwrap().as_str(),
            "첫 방송"
        );
        assert!(BroadcastTitle::parse("a").is_ok());
        assert!(
            BroadcastTitle::parse(&"가".repeat(100)).is_ok(),
            "300 bytes"
        );
        for bad in [String::new(), "   ".into(), "\t\n".into(), "가".repeat(101)] {
            assert_eq!(
                BroadcastTitle::parse(&bad),
                Err(BroadcastError::InvalidTitle),
                "{bad:?}"
            );
        }
        let padded = format!("  {}  ", "a".repeat(100));
        assert!(BroadcastTitle::parse(&padded).is_ok(), "length after trim");
    }

    #[test]
    fn description_allows_empty_up_to_2000_characters() {
        assert_eq!(BroadcastDescription::parse("").unwrap().as_str(), "");
        assert_eq!(
            BroadcastDescription::parse(" x\n").unwrap().as_str(),
            " x\n"
        );
        assert!(BroadcastDescription::parse(&"가".repeat(2000)).is_ok());
        assert_eq!(
            BroadcastDescription::parse(&"a".repeat(2001)),
            Err(BroadcastError::InvalidDescription)
        );
    }

    #[test]
    fn status_round_trips_and_only_ready_and_ended_are_deletable() {
        for status in BroadcastStatus::ALL {
            assert_eq!(BroadcastStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(BroadcastStatus::parse("ready"), None);
        assert_eq!(BroadcastStatus::parse(""), None);

        let deletable: Vec<_> = BroadcastStatus::ALL
            .into_iter()
            .filter(|s| s.is_deletable())
            .collect();
        assert_eq!(deletable, [BroadcastStatus::Ready, BroadcastStatus::Ended]);
    }

    #[test]
    fn every_status_and_event_has_one_outcome() {
        use BroadcastEvent::*;
        use BroadcastStatus::*;
        let table = [
            (Ready, Start, Some(Live)),
            (Ready, Interrupt, None),
            (Ready, Resume, None),
            (Ready, Stop, None),
            (Ready, Expire, None),
            (Live, Start, None),
            (Live, Interrupt, Some(Interrupted)),
            (Live, Resume, None),
            (Live, Stop, Some(Ended)),
            (Live, Expire, None),
            (Interrupted, Start, None),
            (Interrupted, Interrupt, None),
            (Interrupted, Resume, Some(Live)),
            (Interrupted, Stop, Some(Ended)),
            (Interrupted, Expire, Some(Ended)),
            (Ended, Start, None),
            (Ended, Interrupt, None),
            (Ended, Resume, None),
            (Ended, Stop, None),
            (Ended, Expire, None),
        ];
        assert_eq!(
            table.len(),
            BroadcastStatus::ALL.len() * BroadcastEvent::ALL.len()
        );
        for status in BroadcastStatus::ALL {
            for event in BroadcastEvent::ALL {
                let (_, _, next) = table
                    .iter()
                    .find(|(s, e, _)| *s == status && *e == event)
                    .unwrap_or_else(|| panic!("{status:?} x {event:?} missing from the table"));
                assert_eq!(status.apply(event), *next, "{status:?} x {event:?}");
            }
        }
    }

    #[test]
    fn ended_is_final_and_active_is_live_or_interrupted() {
        for event in BroadcastEvent::ALL {
            assert_eq!(BroadcastStatus::Ended.apply(event), None, "{event:?}");
        }
        let active: Vec<_> = BroadcastStatus::ALL
            .into_iter()
            .filter(|s| s.is_active())
            .collect();
        assert_eq!(
            active,
            [BroadcastStatus::Live, BroadcastStatus::Interrupted]
        );
        assert_eq!(
            BroadcastEvent::Stop.transition().from,
            BroadcastStatus::ACTIVE
        );
        assert_eq!(INTERRUPT_GRACE, Duration::from_secs(30));
    }

    #[test]
    fn transition_errors_are_conflicts_with_stable_codes() {
        for (err, code) in [
            (BroadcastError::NotStartable, "BROADCAST_NOT_STARTABLE"),
            (BroadcastError::NotLive, "BROADCAST_NOT_LIVE"),
            (BroadcastError::OwnerAlreadyLive, "OWNER_ALREADY_LIVE"),
        ] {
            assert_eq!(err.kind(), ErrorKind::Conflict, "{code}");
            assert_eq!(err.code(), code);
        }
    }

    #[test]
    fn version_is_positive() {
        assert_eq!(Version::new(1), Some(Version::FIRST));
        assert_eq!(Version::new(7).map(Version::get), Some(7));
        assert_eq!(Version::new(0), None);
        assert_eq!(Version::new(-1), None);
    }

    #[test]
    fn errors_have_stable_codes_and_kinds() {
        assert_eq!(BroadcastError::NotOwner.kind(), ErrorKind::Forbidden);
        assert_eq!(BroadcastError::NotOwner.code(), "NOT_BROADCAST_OWNER");
        assert_eq!(BroadcastError::VersionConflict.kind(), ErrorKind::Conflict);
        assert_eq!(
            BroadcastError::NotDeletable.code(),
            "BROADCAST_NOT_DELETABLE"
        );
        assert_eq!(BroadcastError::NotFound.kind(), ErrorKind::NotFound);
    }
}
