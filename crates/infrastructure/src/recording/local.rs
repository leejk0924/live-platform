use std::{
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use application::ports::{
    PortError,
    media::StreamPath,
    recording::{RecordingStorage, SegmentSpool, SpooledSegment},
};
use async_trait::async_trait;
use domain::recording::RecordingKey;

/// The directory MediaMTX records into, mounted at the same path in the API
/// container, so the path MediaMTX reports is also the API's path.
pub struct FsSegmentSpool {
    root: PathBuf,
}

impl FsSegmentSpool {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn file(&self, segment: &SpooledSegment) -> PathBuf {
        self.root.join(segment.as_str())
    }
}

/// MediaMTX names segments after their start time, such as
/// `2026-11-08_19-32-15-610734.mp4`.
fn is_segment_name(name: &str) -> bool {
    name.strip_suffix(".mp4").is_some_and(|stem| {
        !stem.is_empty()
            && stem
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-'))
    })
}

#[async_trait]
impl SegmentSpool for FsSegmentSpool {
    /// Decided from the reported text alone: anything but plain names
    /// (`..`, a second root, a drive) makes it no segment.
    fn segment(&self, path: &StreamPath, reported: &str) -> Option<SpooledSegment> {
        let relative = Path::new(reported).strip_prefix(&self.root).ok()?;
        let parts = relative
            .components()
            .map(|part| match part {
                Component::Normal(name) => name.to_str(),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()?;
        let (name, dir) = parts.split_last()?;
        let in_path_dir = dir.iter().copied().eq(path.as_str().split('/'));
        (in_path_dir && is_segment_name(name))
            .then(|| SpooledSegment::new(format!("{}/{name}", path.as_str())))
    }

    async fn discard(&self, segment: &SpooledSegment) -> Result<(), PortError> {
        match tokio::fs::remove_file(self.file(segment)).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
            Err(err) => Err(PortError(format!(
                "discard spooled segment {}: {err}",
                segment.as_str()
            ))),
        }
    }
}

/// Keeps recordings in a local directory, served by the API itself. For a
/// single host; [`super::S3RecordingStorage`] is the shared one.
pub struct FsRecordingStorage {
    spool: Arc<FsSegmentSpool>,
    root: PathBuf,
}

impl FsRecordingStorage {
    pub fn new(spool: Arc<FsSegmentSpool>, root: PathBuf) -> Self {
        Self { spool, root }
    }

    /// Where the recording stored under `key` is.
    pub fn file(&self, key: &RecordingKey) -> PathBuf {
        self.root.join(key.as_str())
    }
}

#[async_trait]
impl RecordingStorage for FsRecordingStorage {
    /// Copies to a temporary name next to the target, then renames it into
    /// place, so a reader never sees a partial file and a crash leaves only
    /// a `.part` file behind.
    async fn put(&self, segment: &SpooledSegment, key: &RecordingKey) -> Result<u64, PortError> {
        let error = |what: &str, err: std::io::Error| {
            PortError(format!(
                "store {} as {}: {what}: {err}",
                segment.as_str(),
                key.as_str()
            ))
        };
        let target = self.file(key);
        let dir = target
            .parent()
            .ok_or_else(|| PortError(format!("no directory for {}", key.as_str())))?;
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|err| error("create directory", err))?;
        let mut suffix = [0u8; 8];
        getrandom::fill(&mut suffix).map_err(|err| PortError(format!("os rng failed: {err}")))?;
        let suffix: String = suffix.iter().map(|b| format!("{b:02x}")).collect();
        let temporary = target.with_extension(format!("mp4.{suffix}.part"));
        let copied = async {
            let size = tokio::fs::copy(self.spool.file(segment), &temporary)
                .await
                .map_err(|err| error("copy", err))?;
            tokio::fs::rename(&temporary, &target)
                .await
                .map_err(|err| error("rename", err))?;
            Ok(size)
        }
        .await;
        if copied.is_err() {
            let _ = tokio::fs::remove_file(&temporary).await;
        }
        copied
    }
}

#[cfg(test)]
mod tests {
    use domain::id::SnowflakeId;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let mut suffix = [0u8; 8];
        getrandom::fill(&mut suffix).unwrap();
        let dir = std::env::temp_dir().join(format!("lp-{name}-{}", u64::from_le_bytes(suffix)));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn path(raw: &str) -> StreamPath {
        StreamPath::new(raw.into())
    }

    /// `root` as MediaMTX would write it, followed by `rest`.
    fn reported(root: &Path, rest: &str) -> String {
        format!("{}/{rest}", root.display())
    }

    #[test]
    fn a_segment_directly_in_its_path_directory_is_accepted() {
        let root = temp_dir("spool");
        let spool = FsSegmentSpool::new(root.clone());
        let name = "2026-11-08_19-32-15-610734.mp4";

        let segment = spool
            .segment(
                &path("live/42"),
                &reported(&root, &format!("live/42/{name}")),
            )
            .unwrap();

        assert_eq!(segment.as_str(), format!("live/42/{name}"));
        assert_eq!(spool.file(&segment), root.join("live/42").join(name));
    }

    #[test]
    fn anything_else_is_not_a_segment() {
        let root = temp_dir("spool");
        let spool = FsSegmentSpool::new(root.clone());
        let rejected = [
            "live/42/../42/a.mp4",
            "live/42/../../etc/a.mp4",
            "live/43/a.mp4",
            "live/a.mp4",
            "live/42/sub/a.mp4",
            "live/42/.mp4",
            "live/42/a.mkv",
            "live/42/a b.mp4",
            "live/42/a.mp4.part",
            "live/42",
        ];
        for rest in rejected {
            assert_eq!(
                spool.segment(&path("live/42"), &reported(&root, rest)),
                None,
                "{rest}"
            );
        }
        for outside in [
            "/etc/a.mp4",
            "live/42/a.mp4",
            "",
            "/recordings/live/42/a.mp4",
        ] {
            assert_eq!(spool.segment(&path("live/42"), outside), None, "{outside}");
        }
        assert_eq!(
            spool.segment(&path("live/../x"), &reported(&root, "live/../x/a.mp4")),
            None,
            "a path that leaves its directory"
        );
    }

    #[tokio::test]
    async fn discarding_removes_the_file_and_a_missing_one_is_fine() {
        let root = temp_dir("spool");
        std::fs::create_dir_all(root.join("live/42")).unwrap();
        std::fs::write(root.join("live/42/a.mp4"), b"x").unwrap();
        let spool = FsSegmentSpool::new(root.clone());
        let segment = SpooledSegment::new("live/42/a.mp4".into());

        spool.discard(&segment).await.unwrap();
        assert!(!root.join("live/42/a.mp4").exists());
        spool.discard(&segment).await.unwrap();
    }

    fn key() -> RecordingKey {
        let id = |raw| SnowflakeId::try_from(raw).unwrap();
        RecordingKey::new(id(42), id(7))
    }

    #[tokio::test]
    async fn storing_copies_the_segment_under_its_key_and_can_repeat() {
        let spool_root = temp_dir("spool");
        std::fs::create_dir_all(spool_root.join("live/42")).unwrap();
        std::fs::write(spool_root.join("live/42/a.mp4"), b"segment bytes").unwrap();
        let store_root = temp_dir("store");
        let storage = FsRecordingStorage::new(
            Arc::new(FsSegmentSpool::new(spool_root.clone())),
            store_root.clone(),
        );
        let segment = SpooledSegment::new("live/42/a.mp4".into());

        assert_eq!(storage.put(&segment, &key()).await.unwrap(), 13);
        assert_eq!(storage.put(&segment, &key()).await.unwrap(), 13, "retry");

        let stored = store_root.join("recordings/42/7.mp4");
        assert_eq!(storage.file(&key()), stored);
        assert_eq!(std::fs::read(&stored).unwrap(), b"segment bytes");
        assert!(
            spool_root.join("live/42/a.mp4").exists(),
            "the spool copy stays"
        );
        let leftovers: Vec<_> = std::fs::read_dir(stored.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, ["7.mp4"]);
    }

    #[tokio::test]
    async fn a_missing_segment_is_an_error_that_leaves_nothing_behind() {
        let store_root = temp_dir("store");
        let storage = FsRecordingStorage::new(
            Arc::new(FsSegmentSpool::new(temp_dir("spool"))),
            store_root.clone(),
        );

        let err = storage
            .put(&SpooledSegment::new("live/42/gone.mp4".into()), &key())
            .await
            .unwrap_err();

        assert!(err.0.contains("copy"), "{err}");
        assert_eq!(
            std::fs::read_dir(store_root.join("recordings/42"))
                .unwrap()
                .count(),
            0
        );
    }
}
