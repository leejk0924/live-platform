//! Registering, editing, deleting and browsing broadcasts. Only the
//! authenticated actor is ever passed to the repository as the owner, so a
//! request can never act on someone else's broadcast by naming them.

use std::sync::Arc;

use domain::{
    auth::UserId,
    broadcast::{
        Broadcast, BroadcastDescription, BroadcastError, BroadcastId, BroadcastTitle,
        PAGE_SIZE_DEFAULT, PAGE_SIZE_MAX, Version,
    },
};
use thiserror::Error;

use crate::ports::{
    PortError,
    auth::UserRepository,
    broadcast::{BroadcastQuery, BroadcastRepository, BroadcastView, NewBroadcast, WriteRejection},
    id::{IdGenerationError, IdGenerator},
};

#[derive(Debug, Error)]
pub enum Error {
    #[error(transparent)]
    Rejected(#[from] BroadcastError),
    #[error(transparent)]
    Internal(#[from] PortError),
}

impl From<IdGenerationError> for Error {
    fn from(err: IdGenerationError) -> Self {
        Self::Internal(PortError(err.to_string()))
    }
}

impl From<WriteRejection> for Error {
    fn from(rejection: WriteRejection) -> Self {
        Self::Rejected(match rejection {
            WriteRejection::NotFound => BroadcastError::NotFound,
            WriteRejection::NotOwner => BroadcastError::NotOwner,
            WriteRejection::VersionMismatch => BroadcastError::VersionConflict,
            WriteRejection::NotDeletable => BroadcastError::NotDeletable,
        })
    }
}

/// Writes are only ever made by the owner, so the owner's name is the
/// actor's.
async fn view_of(users: &dyn UserRepository, broadcast: Broadcast) -> Result<BroadcastView, Error> {
    let owner = users
        .find_by_id(broadcast.owner_id)
        .await?
        .ok_or_else(|| PortError(format!("owner of broadcast {} not found", broadcast.id)))?;
    Ok(BroadcastView {
        broadcast,
        owner_name: owner.username,
    })
}

pub struct CreateBroadcast {
    pub ids: Arc<dyn IdGenerator>,
    pub broadcasts: Arc<dyn BroadcastRepository>,
    pub users: Arc<dyn UserRepository>,
}

impl CreateBroadcast {
    pub async fn run(
        &self,
        actor: UserId,
        title: &str,
        description: &str,
    ) -> Result<BroadcastView, Error> {
        let new = NewBroadcast {
            title: BroadcastTitle::parse(title)?,
            description: BroadcastDescription::parse(description)?,
            id: self.ids.next_id()?,
            owner_id: actor,
        };
        let broadcast = self.broadcasts.insert(&new).await?;
        view_of(self.users.as_ref(), broadcast).await
    }
}

pub struct UpdateBroadcast {
    pub broadcasts: Arc<dyn BroadcastRepository>,
    pub users: Arc<dyn UserRepository>,
}

impl UpdateBroadcast {
    pub async fn run(
        &self,
        actor: UserId,
        id: BroadcastId,
        expected: Version,
        title: &str,
        description: &str,
    ) -> Result<BroadcastView, Error> {
        let title = BroadcastTitle::parse(title)?;
        let description = BroadcastDescription::parse(description)?;
        let broadcast = self
            .broadcasts
            .update(id, actor, expected, &title, &description)
            .await??;
        view_of(self.users.as_ref(), broadcast).await
    }
}

pub struct DeleteBroadcast {
    pub broadcasts: Arc<dyn BroadcastRepository>,
}

impl DeleteBroadcast {
    pub async fn run(&self, actor: UserId, id: BroadcastId) -> Result<(), Error> {
        Ok(self.broadcasts.soft_delete(id, actor).await??)
    }
}

pub struct GetBroadcast {
    pub query: Arc<dyn BroadcastQuery>,
}

impl GetBroadcast {
    pub async fn run(&self, id: BroadcastId) -> Result<BroadcastView, Error> {
        self.query
            .get(id)
            .await?
            .ok_or_else(|| BroadcastError::NotFound.into())
    }
}

#[derive(Debug)]
pub struct Page {
    pub items: Vec<BroadcastView>,
    /// Pass as `before` to get the next page; `None` on the last page.
    pub next_before: Option<BroadcastId>,
}

pub struct ListBroadcasts {
    pub query: Arc<dyn BroadcastQuery>,
}

impl ListBroadcasts {
    /// The same for every viewer, so responses may be shared by caches.
    pub async fn run(&self, before: Option<BroadcastId>, size: Option<u32>) -> Result<Page, Error> {
        let size = size.unwrap_or(PAGE_SIZE_DEFAULT);
        if !(1..=PAGE_SIZE_MAX).contains(&size) {
            return Err(BroadcastError::InvalidPageSize.into());
        }
        // One extra row tells whether a next page exists, so the last page
        // never links to an empty one.
        let mut items = self.query.list(before, size + 1).await?;
        let has_more = items.len() > size as usize;
        items.truncate(size as usize);
        let next_before = if has_more {
            items.last().map(|view| view.broadcast.id)
        } else {
            None
        };
        Ok(Page { items, next_before })
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use async_trait::async_trait;
    use domain::{
        auth::Username,
        broadcast::{BroadcastStatus, Version},
        id::SnowflakeId,
    };

    use super::*;

    fn id(raw: i64) -> SnowflakeId {
        SnowflakeId::try_from(raw).unwrap()
    }

    fn view(raw_id: i64) -> BroadcastView {
        BroadcastView {
            broadcast: Broadcast {
                id: id(raw_id),
                owner_id: id(1),
                title: BroadcastTitle::parse("t").unwrap(),
                description: BroadcastDescription::parse("").unwrap(),
                status: BroadcastStatus::Ready,
                version: Version::FIRST,
                created_at: SystemTime::UNIX_EPOCH,
            },
            owner_name: Username::parse("alice").unwrap(),
        }
    }

    /// Holds ids 1..=total and answers like the real query.
    struct Stored(i64);

    #[async_trait]
    impl BroadcastQuery for Stored {
        async fn list(
            &self,
            before: Option<BroadcastId>,
            limit: u32,
        ) -> Result<Vec<BroadcastView>, PortError> {
            let top = before.map_or(self.0, |b| b.as_i64() - 1);
            Ok((1..=top).rev().take(limit as usize).map(view).collect())
        }

        async fn get(&self, _: BroadcastId) -> Result<Option<BroadcastView>, PortError> {
            Ok(None)
        }
    }

    async fn page(total: i64, before: Option<i64>, size: Option<u32>) -> Result<Page, Error> {
        ListBroadcasts {
            query: Arc::new(Stored(total)),
        }
        .run(before.map(id), size)
        .await
    }

    fn ids(page: &Page) -> Vec<i64> {
        page.items.iter().map(|v| v.broadcast.id.as_i64()).collect()
    }

    #[tokio::test]
    async fn next_before_is_set_only_when_another_page_exists() {
        let first = page(5, None, Some(2)).await.unwrap();
        assert_eq!((ids(&first), first.next_before), (vec![5, 4], Some(id(4))));

        let last = page(5, Some(2), Some(2)).await.unwrap();
        assert_eq!((ids(&last), last.next_before), (vec![1], None));

        let exact = page(2, None, Some(2)).await.unwrap();
        assert_eq!((ids(&exact), exact.next_before), (vec![2, 1], None));
    }

    #[tokio::test]
    async fn page_size_defaults_to_20_and_must_be_1_to_50() {
        assert_eq!(page(100, None, None).await.unwrap().items.len(), 20);
        assert_eq!(page(100, None, Some(50)).await.unwrap().items.len(), 50);
        for bad in [0, 51] {
            assert!(
                matches!(
                    page(100, None, Some(bad)).await,
                    Err(Error::Rejected(BroadcastError::InvalidPageSize))
                ),
                "{bad}"
            );
        }
    }
}
