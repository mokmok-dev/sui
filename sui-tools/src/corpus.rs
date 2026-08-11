//! Shared local-code corpus walk for lexical index backends.
//!
//! Policy matches [`crate::bm25::Bm25Index::index_tree`]: extension filter,
//! skip `target` / `node_modules` / dot dirs, size caps, secret-ish skips,
//! no symlink follow.

use std::{collections::HashSet, fs, io::Read, path::Path};

use crate::ToolsError;

/// Maximum bytes read from a single source file during tree indexing.
pub const MAX_FILE_BYTES: u64 = 1_048_576; // 1 MiB
/// Maximum number of documents retained in one index built from a tree walk.
pub const MAX_INDEX_DOCS: usize = 10_000;
/// Preview length (chars) stored on each search hit / document.
pub const SNIPPET_CHARS: usize = 240;

/// Directory names skipped while walking a tree.
const SKIP_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "vendor",
    "__pycache__",
    "dist",
    ".git",
];

/// File name suffixes treated as likely secrets and skipped.
const SKIP_SECRET_SUFFIXES: &[&str] = &[".pem", ".key", ".p12", ".pfx"];
// `.env` and `.env.*` (any suffix) are skipped via [`is_secret_path`].

/// Whether the walk visitor should keep going.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisitControl {
    /// Continue visiting files.
    Continue,
    /// Stop the walk early (e.g. hit [`MAX_INDEX_DOCS`]).
    Stop,
}

/// Builds a short single-line preview from source text.
#[must_use]
pub fn make_snippet(text: &str) -> String {
    let trimmed = text.trim();
    let mut snippet = String::new();
    for (i, ch) in trimmed.chars().enumerate() {
        if i >= SNIPPET_CHARS {
            snippet.push('…');
            break;
        }
        if ch == '\n' || ch == '\r' {
            snippet.push(' ');
        } else {
            snippet.push(ch);
        }
    }
    snippet
}

/// Returns `true` when the path looks like a secret material file.
#[must_use]
pub fn is_secret_path(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    if lower == ".env" || lower.starts_with(".env.") {
        return true;
    }
    SKIP_SECRET_SUFFIXES
        .iter()
        .any(|suffix| lower.ends_with(suffix))
}

fn should_skip_dir(name: &str) -> bool {
    name.starts_with('.') || SKIP_DIRS.contains(&name)
}

/// Walks `root` and invokes `visit` for each UTF-8 text file whose extension is
/// in `extensions` (compared without a leading dot, case-insensitive).
///
/// **Skips** (does not fail the walk):
/// - unreadable files / non-UTF-8 contents
/// - files larger than [`MAX_FILE_BYTES`]
/// - likely secret filenames (`.env`, `.env.*`, `*.pem`, …)
/// - directories in the skip list, and any directory whose name starts with `.`
/// - symlinks (do not follow)
///
/// # Errors
///
/// Returns [`ToolsError::Io`] only when the root directory itself cannot be
/// read. Per-file failures are skipped. Errors from `visit` abort the walk.
pub fn visit_code_files(
    root: &Path,
    extensions: &[&str],
    mut visit: impl FnMut(&Path, &str) -> Result<VisitControl, ToolsError>,
) -> Result<(), ToolsError> {
    let ext_set: HashSet<String> = extensions
        .iter()
        .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
        .collect();
    walk_files(root, &mut |path| {
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            return Ok(VisitControl::Continue);
        };
        if !ext_set.contains(&ext.to_ascii_lowercase()) {
            return Ok(VisitControl::Continue);
        }
        if is_secret_path(path) {
            return Ok(VisitControl::Continue);
        }
        // Bound the read to avoid TOCTOU growth past metadata.len().
        let Ok(file) = fs::File::open(path) else {
            return Ok(VisitControl::Continue);
        };
        let mut limited = file.take(MAX_FILE_BYTES.saturating_add(1));
        let mut bytes = Vec::new();
        if limited.read_to_end(&mut bytes).is_err() {
            return Ok(VisitControl::Continue);
        }
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Ok(VisitControl::Continue);
        }
        let Ok(text) = String::from_utf8(bytes) else {
            return Ok(VisitControl::Continue);
        };
        visit(path, &text)
    })
}

fn walk_files(
    root: &Path,
    visit: &mut dyn FnMut(&Path) -> Result<VisitControl, ToolsError>,
) -> Result<(), ToolsError> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(source) if dir == root => {
                return Err(ToolsError::io(&dir, source));
            },
            Err(_) => continue,
        };
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            // Symlink policy: do not follow (skip symlink dirs/files).
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                let skip = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(should_skip_dir);
                if !skip {
                    stack.push(path);
                }
            } else if file_type.is_file() {
                match visit(&path)? {
                    VisitControl::Continue => {},
                    VisitControl::Stop => return Ok(()),
                }
            }
        }
    }
    Ok(())
}

/// Unique temp directory under the system temp dir (for tests).
#[cfg(test)]
pub(crate) fn temp_dir(label: &str) -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let dir = std::env::temp_dir().join(format!(
        "sui-tools-corpus-{label}-{}-{nanos}-{seq}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

#[cfg(test)]
pub(crate) struct TempDir(pub std::path::PathBuf);

#[cfg(test)]
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_secret_path_matches_env_glob() {
        assert!(is_secret_path(Path::new("/x/.env")));
        assert!(is_secret_path(Path::new("/x/.env.local")));
        assert!(is_secret_path(Path::new("/x/.env.staging")));
        assert!(is_secret_path(Path::new("/x/cert.pem")));
        assert!(!is_secret_path(Path::new("/x/env.rs")));
        assert!(!is_secret_path(Path::new("/x/.environment")));
    }
}
