//! Cross-process exclusion shared with devshell's Cargo wrapper. Never waits.
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::{
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

pub fn lock_path() -> io::Result<PathBuf> {
    let uid = std::fs::metadata("/proc/self")?.uid();
    Ok(PathBuf::from(format!(
        "/tmp/ubu-planning-build-worker-{uid}.lock"
    )))
}
/// The open descriptor owns the advisory lock until Drop, including unwinding.
pub struct ComputeGuard {
    _file: File,
}
impl Drop for ComputeGuard {
    fn drop(&mut self) {
        // An unrelated concurrent fork may briefly inherit the descriptor
        // before exec closes it. Explicit unlock still ends our ownership now.
        let _ = self._file.unlock();
    }
}
impl ComputeGuard {
    pub fn try_acquire() -> io::Result<Self> {
        Self::at(&lock_path()?)
    }
    fn at(path: &Path) -> io::Result<Self> {
        // Linux O_NOFOLLOW prevents accepting a symlink planted in /tmp.
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(0x20000)
            .open(path)?;
        if file.metadata()?.uid() != std::fs::metadata("/proc/self")?.uid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "lock owner mismatch",
            ));
        }
        file.try_lock().map_err(|error| match error {
            std::fs::TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::WouldBlock,
                "build or compute session holds lock",
            ),
            std::fs::TryLockError::Error(error) => error,
        })?;
        Ok(Self { _file: file })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contention_is_immediate_and_drop_and_panic_release() {
        let path =
            std::env::temp_dir().join(format!("ubu-compute-lock-test-{}", std::process::id()));
        let first = ComputeGuard::at(&path).unwrap();
        let start = std::time::Instant::now();
        assert_eq!(
            ComputeGuard::at(&path).err().unwrap().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        drop(first);
        assert!(std::panic::catch_unwind(|| {
            let _guard = ComputeGuard::at(&path).unwrap();
            panic!("synthetic lock owner panic");
        })
        .is_err());
        drop(ComputeGuard::at(&path).unwrap());
        std::fs::remove_file(path).unwrap();
    }
}
