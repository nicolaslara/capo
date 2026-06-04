//! Self-cleaning temporary directories for Capo's test suites.
//!
//! Capo's tests create scratch directories under the OS temp dir (named with a
//! timestamp so parallel tests don't collide) and historically never removed
//! them. Across many `cargo test --workspace` runs this leaked hundreds of
//! thousands of directories / tens of gigabytes into the temp dir.
//!
//! [`TempRoot`] fixes that at the source: it owns a uniquely-named path under
//! [`std::env::temp_dir`] and removes the whole tree on drop. It derefs to
//! [`Path`], so existing call sites that wrote `root.join(..)`, `&root`, or
//! `root.display()` keep compiling unchanged — only the helper's return type
//! changes from `PathBuf` to `TempRoot`.
//!
//! It deliberately does NOT create the directory: callers keep their existing
//! `create_dir_all` calls, preserving the old "path handed out, caller decides"
//! semantics. Cleanup ignores a missing directory.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A uniquely-named temp path that deletes its directory tree when dropped.
///
/// Bind it to a `let` for the lifetime of the test (`let root = TempRoot::new(..)`);
/// the directory survives until the binding goes out of scope. Use [`TempRoot::keep`]
/// to defuse cleanup, or [`TempRoot::to_path_buf`] to hand an owned [`PathBuf`] to a
/// shorter-lived consumer while the guard still owns cleanup.
#[derive(Debug)]
pub struct TempRoot {
    path: PathBuf,
}

impl TempRoot {
    /// Allocate a uniquely-named (but not-yet-created) temp path with `prefix`.
    ///
    /// Uniqueness comes from the process id, a nanosecond timestamp, and a
    /// process-global counter, so concurrent tests within and across processes
    /// never collide.
    #[must_use]
    pub fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("{prefix}-{pid}-{nanos}-{counter}"));
        Self { path }
    }

    /// Wrap an already-chosen path so it is cleaned up on drop.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// Borrow the owned path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Clone the owned path. Cleanup still belongs to this guard.
    #[must_use]
    pub fn to_path_buf(&self) -> PathBuf {
        self.path.clone()
    }

    /// Consume the guard, returning the path and disabling auto-cleanup.
    #[must_use]
    pub fn keep(mut self) -> PathBuf {
        std::mem::take(&mut self.path)
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        // Usually a directory tree, but some tests deliberately create a regular
        // file at the root path (e.g. to force a "not a directory" error). Remove
        // either; ignore a missing path.
        if std::fs::remove_dir_all(&self.path).is_err() {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

// Deref to `PathBuf` (not `Path`) is deliberate: it lets existing call sites keep
// writing `root.clone()` (resolving to `PathBuf::clone`, returning an owned
// `PathBuf`), `root.join(..)`, `root.display()`, and `&root` (coerced to `&Path`)
// with no edits. Only `&self` access is exposed, so the path can't be mutated.
impl std::ops::Deref for TempRoot {
    type Target = PathBuf;

    fn deref(&self) -> &PathBuf {
        &self.path
    }
}

impl AsRef<Path> for TempRoot {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl std::borrow::Borrow<Path> for TempRoot {
    fn borrow(&self) -> &Path {
        &self.path
    }
}

impl From<TempRoot> for PathBuf {
    fn from(root: TempRoot) -> Self {
        root.keep()
    }
}

// Let tests compare a `TempRoot` against owned/borrowed paths from either side
// without reaching for `.path()`.
impl PartialEq<PathBuf> for TempRoot {
    fn eq(&self, other: &PathBuf) -> bool {
        self.path == *other
    }
}

impl PartialEq<TempRoot> for PathBuf {
    fn eq(&self, other: &TempRoot) -> bool {
        *self == other.path
    }
}

impl PartialEq<Path> for TempRoot {
    fn eq(&self, other: &Path) -> bool {
        self.path.as_path() == other
    }
}

impl PartialEq<TempRoot> for Path {
    fn eq(&self, other: &TempRoot) -> bool {
        self == other.path.as_path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_directory_tree_on_drop() {
        let created;
        {
            let root = TempRoot::new("capo-tmptest-drop");
            created = root.to_path_buf();
            std::fs::create_dir_all(root.join("nested")).unwrap();
            std::fs::write(root.join("nested/file.txt"), b"hi").unwrap();
            assert!(created.exists());
        }
        assert!(!created.exists(), "temp tree should be gone after drop");
    }

    #[test]
    fn keep_defuses_cleanup() {
        let kept = {
            let root = TempRoot::new("capo-tmptest-keep");
            std::fs::create_dir_all(&root).unwrap();
            root.keep()
        };
        assert!(kept.exists(), "kept directory must survive the guard");
        std::fs::remove_dir_all(&kept).unwrap();
    }

    #[test]
    fn paths_are_unique() {
        let a = TempRoot::new("capo-tmptest-unique");
        let b = TempRoot::new("capo-tmptest-unique");
        assert_ne!(a.path(), b.path());
    }
}
