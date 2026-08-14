//! Safe application of one-file Git unified diffs.
//!
//! The edit protocol is deliberately line-oriented: the caller supplies a
//! complete patch, `gitpatch` validates its hunk headers, and Git applies the
//! patch after a durable write-ahead check.  No search-and-replace or direct
//! file rewrite is performed here.

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use gitpatch::Patch;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Tool, ToolFuture, ToolsError};

static PATCH_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Validates a single-file unified diff.
///
/// # Errors
///
/// Returns [`ToolsError::Edit`] when the diff is empty, malformed, or has
/// hunk line counts that do not match its header.
pub fn validate_unified_diff(response: &str) -> Result<(), ToolsError> {
    if response.trim().is_empty() {
        return Err(ToolsError::Edit("patch is empty".into()));
    }
    parse_unified_diff(response)?;
    validate_hunk_counts(response)
}

fn parse_unified_diff(response: &str) -> Result<Patch<'_>, ToolsError> {
    Patch::from_single(response)
        .map_err(|error| ToolsError::Edit(format!("invalid unified diff: {error}")))
}

fn parse_hunk_range(value: &str) -> Result<usize, ToolsError> {
    let value = value
        .strip_prefix(['-', '+'])
        .ok_or_else(|| ToolsError::Edit(format!("invalid hunk range `{value}`")))?;
    let (_, count) = value.split_once(',').map_or((value, "1"), |parts| parts);
    count
        .parse()
        .map_err(|_| ToolsError::Edit(format!("invalid hunk count `{count}`")))
}

fn validate_hunk_counts(response: &str) -> Result<(), ToolsError> {
    let mut lines = response.lines().peekable();
    let mut hunks = 0;
    while let Some(line) = lines.next() {
        if !line.starts_with("@@ ") {
            continue;
        }
        let body = line
            .strip_prefix("@@ ")
            .and_then(|body| body.split_once(" @@").map(|(body, _)| body))
            .ok_or_else(|| ToolsError::Edit(format!("invalid hunk header `{line}`")))?;
        let mut ranges = body.split_whitespace();
        let old_count = parse_hunk_range(
            ranges
                .next()
                .ok_or_else(|| ToolsError::Edit("hunk is missing old range".into()))?,
        )?;
        let new_count = parse_hunk_range(
            ranges
                .next()
                .ok_or_else(|| ToolsError::Edit("hunk is missing new range".into()))?,
        )?;
        if ranges.next().is_some() {
            return Err(ToolsError::Edit(format!("invalid hunk header `{line}`")));
        }
        let mut seen_old = 0;
        let mut seen_new = 0;
        while let Some(next) = lines.peek().copied() {
            if next.starts_with("@@ ") || next.starts_with("diff --git ") {
                break;
            }
            let next = lines.next().unwrap_or_default();
            match next.as_bytes().first().copied() {
                Some(b' ') => {
                    seen_old += 1;
                    seen_new += 1;
                },
                Some(b'-') => seen_old += 1,
                Some(b'+') => seen_new += 1,
                Some(b'\\') => {},
                _ => {
                    return Err(ToolsError::Edit(format!("invalid line in hunk `{next}`")));
                },
            }
        }
        if seen_old != old_count || seen_new != new_count {
            return Err(ToolsError::Edit(format!(
                "hunk header counts {old_count}/{new_count}, found {seen_old}/{seen_new}"
            )));
        }
        hunks += 1;
    }
    if hunks == 0 {
        return Err(ToolsError::Edit("patch contains no hunks".into()));
    }
    Ok(())
}

fn git_output(command: &mut Command) -> Result<std::process::Output, ToolsError> {
    command
        .output()
        .map_err(|source| ToolsError::io("git", source))
}

fn repository_root(file: &Path) -> Result<PathBuf, ToolsError> {
    let absolute = absolute_target_path(file)?;
    let mut probe = absolute
        .parent()
        .ok_or_else(|| ToolsError::Edit("target has no parent directory".into()))?;
    while !probe.exists() {
        probe = probe
            .parent()
            .ok_or_else(|| ToolsError::Edit("target has no existing parent directory".into()))?;
    }
    let probe = std::fs::canonicalize(probe).map_err(|source| ToolsError::io(probe, source))?;
    let output = git_output(
        Command::new("git")
            .arg("-C")
            .arg(probe)
            .args(["rev-parse", "--show-toplevel"]),
    )?;
    if !output.status.success() {
        return Err(ToolsError::Edit(format!(
            "target is not inside a Git worktree: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let root = String::from_utf8(output.stdout)
        .map_err(|error| ToolsError::Edit(format!("git returned invalid path: {error}")))?;
    let root = PathBuf::from(root.trim());
    std::fs::canonicalize(&root).map_err(|source| ToolsError::io(&root, source))
}

fn absolute_target_path(file: &Path) -> Result<PathBuf, ToolsError> {
    if file.is_absolute() {
        Ok(file.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|source| ToolsError::io("current directory", source))?
            .join(file))
    }
}

fn relative_target(
    root: &Path,
    file: &Path,
) -> Result<PathBuf, ToolsError> {
    let absolute = absolute_target_path(file)?;
    let absolute = if absolute.exists() {
        std::fs::canonicalize(&absolute).map_err(|source| ToolsError::io(&absolute, source))?
    } else {
        let mut missing = Vec::new();
        let mut probe = absolute.as_path();
        while !probe.exists() {
            missing.push(
                probe
                    .file_name()
                    .ok_or_else(|| ToolsError::Edit("target has no file name".into()))?,
            );
            probe = probe.parent().ok_or_else(|| {
                ToolsError::Edit("target has no existing parent directory".into())
            })?;
        }
        let mut resolved =
            std::fs::canonicalize(probe).map_err(|source| ToolsError::io(probe, source))?;
        for component in missing.into_iter().rev() {
            resolved.push(component);
        }
        resolved
    };
    let relative = absolute
        .strip_prefix(root)
        .map_err(|_| ToolsError::Edit("target must be inside the Git worktree".into()))?;
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        })
    {
        return Err(ToolsError::Edit(
            "target must be a file below the worktree".into(),
        ));
    }
    Ok(relative.to_path_buf())
}

fn header_path(path: &str) -> Option<&str> {
    match path {
        "/dev/null" => None,
        path if path.starts_with("a/") || path.starts_with("b/") => Some(&path[2..]),
        path => Some(path),
    }
}

fn check_target(
    root: &Path,
    file: &Path,
    response: &str,
) -> Result<(), ToolsError> {
    let target = relative_target(root, file)?;
    let patch = parse_unified_diff(response)?;
    let old = header_path(&patch.old.path);
    let new = header_path(&patch.new.path);
    if old != Some(target.to_string_lossy().as_ref())
        && new != Some(target.to_string_lossy().as_ref())
    {
        return Err(ToolsError::Edit(format!(
            "patch does not target requested file `{}`",
            target.display()
        )));
    }
    if old.is_some() && new.is_some() && old != new {
        return Err(ToolsError::Edit("renames are not supported by edit".into()));
    }
    Ok(())
}

fn write_ahead_patch(response: &str) -> Result<PathBuf, ToolsError> {
    let sequence = PATCH_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("sui-edit-{}-{sequence}.patch", std::process::id()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(&path)
        .map_err(|source| ToolsError::io(&path, source))?;
    let write_result = file
        .write_all(response.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| ToolsError::io(&path, source));
    if let Err(error) = write_result {
        let _ = std::fs::remove_file(&path);
        return Err(error);
    }
    Ok(path)
}

fn apply_patch(
    root: &Path,
    patch: &Path,
) -> Result<(), ToolsError> {
    let check = git_output(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["apply", "--check", "--whitespace=nowarn"])
            .arg(patch),
    )?;
    if !check.status.success() {
        return Err(ToolsError::Edit(format!(
            "git apply --check rejected patch: {}",
            String::from_utf8_lossy(&check.stderr).trim()
        )));
    }
    let applied = git_output(
        Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["apply", "--whitespace=nowarn"])
            .arg(patch),
    )?;
    if !applied.status.success() {
        return Err(ToolsError::Edit(format!(
            "git apply failed after check: {}",
            String::from_utf8_lossy(&applied.stderr).trim()
        )));
    }
    Ok(())
}

/// Applies one validated unified diff to the requested file through Git.
#[derive(Default)]
pub struct EditTool;

impl EditTool {
    /// Creates an [`EditTool`].
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Tool for EditTool {
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "edit"
    }

    #[allow(clippy::unnecessary_literal_bound)]
    fn description(&self) -> &str {
        "Apply one Git unified diff to the requested file. The patch must contain valid ---/+++ headers and hunk counts, and its path must match `file`. For a new file use --- /dev/null and +++ b/path; for deletion use --- a/path and +++ /dev/null. Renames and multi-file patches are rejected. The patch is written ahead, checked, and applied by git."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file": { "type": "string", "description": "Requested target path; must match the patch header" },
                "response": { "type": "string", "description": "One Git unified diff. New files: --- /dev/null and +++ b/path. Deletions: --- a/path and +++ /dev/null." }
            },
            "required": ["file", "response"],
            "additionalProperties": false
        })
    }

    fn call(
        &self,
        args: Value,
    ) -> ToolFuture<'_> {
        Box::pin(async move {
            let args: EditArgs = serde_json::from_value(args)
                .map_err(|error| ToolsError::InvalidArgs(error.to_string()))?;
            validate_unified_diff(&args.response)?;
            let root = repository_root(&args.file)?;
            check_target(&root, &args.file, &args.response)?;
            let patch = write_ahead_patch(&args.response)?;
            let result = apply_patch(&root, &patch);
            let _ = std::fs::remove_file(&patch);
            result?;
            Ok(json!({ "file": args.file, "changed": true, "applied": true }))
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EditArgs {
    file: PathBuf,
    response: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolRegistry;
    use crate::corpus::{TempDir, temp_dir};
    use serde_json::json;
    use std::fs;

    const PATCH: &str = "diff --git a/sample.rs b/sample.rs\n--- a/sample.rs\n+++ b/sample.rs\n@@ -1,2 +1,2 @@\n-fn old() {}\n+fn new() {}\n fn main() {}\n";

    #[test]
    fn validates_hunk_headers() -> Result<(), ToolsError> {
        validate_unified_diff(PATCH)
    }

    #[test]
    fn rejects_invalid_hunk_counts() {
        let error = validate_unified_diff("--- a/x\n+++ b/x\n@@ -1,2 +1,2 @@\n-old\n+new\n")
            .expect_err("incomplete hunk");
        assert!(error.to_string().contains("hunk header counts"));
    }

    #[test]
    fn recognizes_dev_null_create_and_delete_paths() {
        assert_eq!(header_path("/dev/null"), None);
        assert_eq!(header_path("b/new.rs"), Some("new.rs"));
        assert_eq!(header_path("a/old.rs"), Some("old.rs"));
    }

    #[test]
    fn accepts_quoted_paths() -> Result<(), ToolsError> {
        validate_unified_diff(
            "--- \"a/old name.rs\"\n+++ \"b/old name.rs\"\n@@ -1 +1 @@\n-old\n+new\n",
        )
    }

    #[tokio::test]
    async fn applies_create_and_delete_through_git() -> Result<(), ToolsError> {
        let dir = TempDir(temp_dir("edit-git"));
        fs::create_dir_all(&dir.0).map_err(|source| ToolsError::io(&dir.0, source))?;
        let init = Command::new("git")
            .arg("-C")
            .arg(&dir.0)
            .args(["init", "-q"])
            .status()
            .map_err(|source| ToolsError::io("git", source))?;
        assert!(init.success());

        let mut registry = ToolRegistry::new();
        registry.register(EditTool);
        let file = dir.0.join("nested").join("new.rs");
        registry
            .call(
                "edit",
                json!({
                    "file": file,
                    "response": "diff --git a/nested/new.rs b/nested/new.rs\nnew file mode 100644\n--- /dev/null\n+++ b/nested/new.rs\n@@ -0,0 +1 @@\n+fn new() {}\n"
                }),
            )
            .await?;
        assert_eq!(
            fs::read_to_string(&file).map_err(|source| ToolsError::io(&file, source))?,
            "fn new() {}\n"
        );

        registry
            .call(
                "edit",
                json!({
                    "file": file,
                    "response": "diff --git a/nested/new.rs b/nested/new.rs\ndeleted file mode 100644\n--- a/nested/new.rs\n+++ /dev/null\n@@ -1 +0,0 @@\n-fn new() {}\n"
                }),
            )
            .await?;
        assert!(!file.exists());
        Ok(())
    }
}
