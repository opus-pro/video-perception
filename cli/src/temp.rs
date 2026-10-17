//! Private temporary directories for intermediate files.
//!
//! `std::env::temp_dir()` is shared with every process on the machine, so a
//! predictable file name there can be pre-created, or replaced by a symbolic
//! link, before this process writes to it. A `PrivateDir` is created
//! atomically under a unique name with mode `0700`, every intermediate file
//! lives inside it, and the directory is removed when the value drops.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const ATTEMPTS: u32 = 64;

pub struct PrivateDir {
    path: PathBuf,
}

impl PrivateDir {
    /// Create a fresh directory under the system temporary directory. The
    /// name carries `purpose` so a leftover directory is attributable.
    pub fn create(purpose: &str) -> Result<Self, String> {
        let base = std::env::temp_dir();
        for _ in 0..ATTEMPTS {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or(0);
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!(
                "video-perception-{purpose}-{}-{nonce}-{sequence}",
                std::process::id()
            ));
            match create_private(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(format!(
                        "create private temporary directory {}: {error}",
                        path.display()
                    ));
                }
            }
        }
        Err(format!(
            "could not create a private temporary directory under {} after {ATTEMPTS} attempts",
            base.display()
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A path for a file inside the directory. Nothing is created.
    pub fn file(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// `mkdir` without following a symbolic link at the final component: an
/// existing entry of any kind makes it fail with `AlreadyExists`, which the
/// caller treats as "pick another name".
#[cfg(unix)]
fn create_private(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> io::Result<()> {
    fs::DirBuilder::new().create(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_unique_directories_and_removes_them_on_drop() {
        let first = PrivateDir::create("test").unwrap();
        let second = PrivateDir::create("test").unwrap();
        assert_ne!(first.path(), second.path());
        assert!(first.path().is_dir());
        assert!(second.path().is_dir());
        let kept = first.path().to_path_buf();
        fs::write(first.file("scratch.txt"), b"x").unwrap();
        drop(first);
        assert!(
            !kept.exists(),
            "directory and its contents are removed on drop"
        );
    }

    #[cfg(unix)]
    #[test]
    fn directory_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = PrivateDir::create("perm").unwrap();
        let mode = fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn file_paths_stay_inside_the_directory() {
        let dir = PrivateDir::create("file").unwrap();
        assert_eq!(dir.file("audio.wav"), dir.path().join("audio.wav"));
    }
}
