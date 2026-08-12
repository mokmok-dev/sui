//! Search-and-replace edit blocks from LLM replies.
//!
//! An agent that reads code and wants to change it can respond with one or
//! more `SEARCH` / `REPLACE` blocks:
//!
//! ```text
//! <<<< SEARCH
//! <existing code, matched byte-for-byte>
//! ====
//! <replacement code>
//! >>>> REPLACE
//! ```
//!
//! [`parse_search_replace_blocks`] extracts those blocks from free-form reply
//! text using a single regular expression, and [`apply_blocks`] applies them to
//! file content. [`EditTool`] wires both together behind the [`Tool`] trait:
//! given a target `file` and the raw `response`, it atomically rewrites the
//! file — if any block fails to apply, nothing is written.
//!
//! # Matching semantics
//!
//! Matching is **byte-exact and whitespace-sensitive**, and every block must
//! match **exactly one** location. This is deliberate: a non-unique or fuzzy
//! match is a guess, and a silent wrong edit is worse than a loud error. When
//! a block is ambiguous or absent, the error names the block and previews its
//! SEARCH text so the caller can regenerate a more precise block.
//!
//! Blocks apply **sequentially**: each block searches inside the result of the
//! previous one. Content that is identical to the original is detected and the
//! file is left untouched.
//!
//! # Threat model
//!
//! Like [`crate::tool::BashTool`], [`EditTool`] is **not a sandbox**: it writes
//! to whatever path the caller passes with the host process credentials.
//! Callers must gate tool exposure when untrusted agents can invoke it.

use std::{path::PathBuf, sync::LazyLock};

use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{Tool, ToolFuture, ToolsError};

/// Regex matching a `<<…< SEARCH` / `>>…> REPLACE` block.
///
/// - `<<+` / `>>+` accept two or more fence characters (`<<<< SEARCH`).
/// - `(?s)` lets `.*?` span newlines; both groups are non-greedy, so each
///   capture stops at the first following separator line. Captures exclude the
///   newline boundaries, so a well-formed block yields exactly the code text.
const SEARCH_REPLACE_PATTERN: &str =
    r"(?s)<<+ SEARCH\n(?P<search>.*?)\n===+\n(?P<replace>.*?)\n>>+ REPLACE";

/// Compiled once; failures are impossible for the constant pattern but are
/// still surfaced instead of panicking (workspace forbids `expect`/`unwrap`).
static SEARCH_REPLACE_RE: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(SEARCH_REPLACE_PATTERN));

/// Max length of a SEARCH preview embedded in a "not found" error.
const SEARCH_PREVIEW_CHARS: usize = 80;

/// One `SEARCH` / `REPLACE` block parsed from an LLM reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchReplaceBlock {
    /// Existing code to locate (matched byte-for-byte).
    pub search: String,
    /// Code that replaces the matched `search` text.
    pub replace: String,
}

/// Extracts every `SEARCH` / `REPLACE` block from `response`.
///
/// Non-block prose is ignored, so the reply may contain markup, reasoning, or
/// multiple blocks around the edits. Carriage returns are normalized away
/// (`\r\n` → `\n`) so replies copied from CRLF sources still parse, and the
/// resulting SEARCH text matches files that use LF endings. SEARCH must still
/// match CRLF *files* byte-for-byte — convert such files to LF first.
///
/// # Errors
///
/// Returns [`ToolsError::Regex`] if the compiled pattern cannot be run (only
/// possible if the static pattern is changed to something invalid).
pub fn parse_search_replace_blocks(response: &str) -> Result<Vec<SearchReplaceBlock>, ToolsError> {
    // Only allocate when a `\r` is actually present.
    let normalized = response
        .contains('\r')
        .then(|| response.replace("\r\n", "\n"));
    let response = normalized.as_deref().unwrap_or(response);
    let re = SEARCH_REPLACE_RE
        .as_ref()
        .map_err(|error| ToolsError::Regex(error.to_string()))?;
    let mut blocks = Vec::new();
    for captures in re.captures_iter(response) {
        let search = captures
            .name("search")
            .ok_or_else(|| ToolsError::Regex("pattern lacks `search` capture".into()))?;
        let replace = captures
            .name("replace")
            .ok_or_else(|| ToolsError::Regex("pattern lacks `replace` capture".into()))?;
        blocks.push(SearchReplaceBlock {
            search: search.as_str().to_owned(),
            replace: replace.as_str().to_owned(),
        });
    }
    Ok(blocks)
}

/// Applies `blocks` to `content`, returning the fully updated text.
///
/// All-or-nothing: returns an error on the first block that cannot be applied,
/// leaving `content` untouched (callers must only persist on success).
///
/// # Errors
///
/// Returns [`ToolsError::Edit`] when a block has empty SEARCH text, when the
/// SEARCH text appears zero times (byte-exact), or when it appears more than
/// once (ambiguous).
pub fn apply_blocks(
    content: &str,
    blocks: &[SearchReplaceBlock],
) -> Result<String, ToolsError> {
    let mut result = content.to_owned();
    for (index, block) in blocks.iter().enumerate() {
        let block_num = index + 1;
        let preview = block
            .search
            .chars()
            .take(SEARCH_PREVIEW_CHARS)
            .collect::<String>();
        if block.search.is_empty() {
            return Err(block_error(block_num, "SEARCH text is empty"));
        }
        let start = {
            let mut matches = result.match_indices(&block.search);
            let (start, _) = matches.next().ok_or_else(|| {
                block_error(
                    block_num,
                    &format!("SEARCH text not found in file; preview: {preview:?}"),
                )
            })?;
            if matches.next().is_some() {
                return Err(block_error(
                    block_num,
                    &format!(
                        "SEARCH text matches more than one location; preview: {preview:?} — \
                         add surrounding context to make it unique"
                    ),
                ));
            }
            start
        };
        result.replace_range(start..start + block.search.len(), &block.replace);
    }
    Ok(result)
}

fn block_error(
    block_num: usize,
    detail: &str,
) -> ToolsError {
    ToolsError::Edit(format!("block {block_num}: {detail}"))
}

/// Applies `SEARCH` / `REPLACE` blocks from an LLM reply to a single file.
///
/// # Arguments
///
/// - `file` (string, required): path of the file to edit
/// - `response` (string, required): reply text containing one or more
///   `<<<< SEARCH … ==== … >>>> REPLACE` blocks
///
/// Blocks are matched byte-exactly and must be unique; they apply sequentially
/// and atomically — if any block fails, the file is left unchanged.
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
        "Apply SEARCH/REPLACE blocks from a reply to a file. Blocks look like \
         `<<<< SEARCH\\n<exact existing code>\\n====\\n<replacement>\\n>>>> REPLACE`; \
         SEARCH must match exactly one location (whitespace-sensitive). All blocks apply atomically."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file": {
                    "type": "string",
                    "description": "Path of the file to edit"
                },
                "response": {
                    "type": "string",
                    "description": "Reply containing one or more `<<<< SEARCH ... ==== ... >>>> REPLACE` blocks"
                }
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
            let blocks = parse_search_replace_blocks(&args.response)?;
            if blocks.is_empty() {
                return Err(ToolsError::InvalidArgs(
                    "response contains no SEARCH/REPLACE blocks".into(),
                ));
            }
            let content = std::fs::read_to_string(&args.file)
                .map_err(|source| ToolsError::io(&args.file, source))?;
            let updated = apply_blocks(&content, &blocks)?;
            let changed = updated != content;
            if changed {
                std::fs::write(&args.file, &updated)
                    .map_err(|source| ToolsError::io(&args.file, source))?;
            }
            Ok(json!({
                "file": args.file,
                "blocks_applied": blocks.len(),
                "changed": changed,
            }))
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
    use serde_json::json;
    use std::fs;

    use crate::ToolRegistry;
    use crate::corpus::{TempDir, temp_dir};

    const TWO_BLOCK_REPLY: &str = "\
Here is the patch:

<<<< SEARCH
fn legacy(&self) -> usize {
    self.items.len()
}
====
fn modern(&self) -> usize {
    self.items.iter().filter(|item| item.enabled).count()
}
>>>> REPLACE

Let me also change the caller:

<<<< SEARCH
legacy()
====
modern()
>>>> REPLACE

Done.";

    #[test]
    fn parses_blocks_ignoring_surrounding_prose() -> Result<(), ToolsError> {
        let blocks = parse_search_replace_blocks(TWO_BLOCK_REPLY)?;
        assert_eq!(blocks.len(), 2);
        assert_eq!(
            blocks[0].search,
            "fn legacy(&self) -> usize {\n    self.items.len()\n}"
        );
        assert_eq!(
            blocks[0].replace,
            "fn modern(&self) -> usize {\n    self.items.iter().filter(|item| item.enabled).count()\n}"
        );
        assert_eq!(blocks[1].search, "legacy()");
        assert_eq!(blocks[1].replace, "modern()");
        Ok(())
    }

    #[test]
    fn parses_variable_fence_widths() -> Result<(), ToolsError> {
        let blocks = parse_search_replace_blocks(
            "prefix\n<< SEARCH\nold\n=====\nnew\n>>>>>> REPLACE\nsuffix",
        )?;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "old");
        assert_eq!(blocks[0].replace, "new");
        Ok(())
    }

    #[test]
    fn parses_multiline_search_and_replace() -> Result<(), ToolsError> {
        let blocks =
            parse_search_replace_blocks("<<<< SEARCH\na\nb\nc\n====\nd\ne\nf\n>>>> REPLACE")?;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "a\nb\nc");
        assert_eq!(blocks[0].replace, "d\ne\nf");
        Ok(())
    }

    #[test]
    fn no_blocks_yields_empty() -> Result<(), ToolsError> {
        assert!(parse_search_replace_blocks("no edit blocks here")?.is_empty());
        assert!(parse_search_replace_blocks("")?.is_empty());
        Ok(())
    }

    #[test]
    fn normalizes_crlf_in_response() -> Result<(), ToolsError> {
        let blocks = parse_search_replace_blocks(
            "<<<< SEARCH\r\nfn old() {}\r\n====\r\nfn new() {}\r\n>>>> REPLACE",
        )?;
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "fn old() {}");
        assert_eq!(blocks[0].replace, "fn new() {}");
        Ok(())
    }

    #[test]
    fn missing_separator_or_fence_is_not_a_block() -> Result<(), ToolsError> {
        let blocks = parse_search_replace_blocks(
            "<<<< SEARCH\nold\n===\nnew\n>>>> REPLACE\n<<<< SEARCH\nold2\n====\nnew2",
        )?;
        // First block parses; the tail without a closing fence does not.
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "old");
        assert_eq!(blocks[0].replace, "new");
        Ok(())
    }

    #[test]
    fn applies_single_block() -> Result<(), ToolsError> {
        let content = "fn legacy() {}\n\nfn main() {}";
        let updated = apply_blocks(
            content,
            &[SearchReplaceBlock {
                search: "fn legacy() {}".into(),
                replace: "fn modern() {}".into(),
            }],
        )?;
        assert_eq!(updated, "fn modern() {}\n\nfn main() {}");
        Ok(())
    }

    #[test]
    fn applies_blocks_sequentially() -> Result<(), ToolsError> {
        let content = "one two three";
        let updated = apply_blocks(
            content,
            &[
                SearchReplaceBlock {
                    search: "one".into(),
                    replace: "uno".into(),
                },
                SearchReplaceBlock {
                    // Later blocks see the earlier replacement.
                    search: "uno".into(),
                    replace: "ichi".into(),
                },
            ],
        )?;
        assert_eq!(updated, "ichi two three");
        Ok(())
    }

    #[test]
    fn unmatched_search_errors_and_preserves_content() {
        let content = "fn other() {}";
        let err = apply_blocks(
            content,
            &[SearchReplaceBlock {
                search: "fn missing() {}".into(),
                replace: "fn present() {}".into(),
            }],
        )
        .expect_err("missing search");
        assert!(matches!(err, ToolsError::Edit(_)));
        assert!(err.to_string().contains("not found"), "{err}");
        assert!(err.to_string().contains("fn missing"), "{err}");
    }

    #[test]
    fn ambiguous_search_errors() {
        let content = "dup\ndup\ndup";
        let err = apply_blocks(
            content,
            &[SearchReplaceBlock {
                search: "dup".into(),
                replace: "one".into(),
            }],
        )
        .expect_err("ambiguous");
        assert!(matches!(err, ToolsError::Edit(_)));
        assert!(err.to_string().contains("more than one"), "{err}");
    }

    #[test]
    fn empty_search_errors() {
        let err = apply_blocks(
            "anything",
            &[SearchReplaceBlock {
                search: String::new(),
                replace: "x".into(),
            }],
        )
        .expect_err("empty search");
        assert!(matches!(err, ToolsError::Edit(_)));
        assert!(err.to_string().contains("empty"), "{err}");
    }

    #[test]
    fn later_block_failure_aborts_whole_apply() {
        let content = "a b";
        let err = apply_blocks(
            content,
            &[
                SearchReplaceBlock {
                    search: "a".into(),
                    replace: "x".into(),
                },
                SearchReplaceBlock {
                    search: "nope".into(),
                    replace: "y".into(),
                },
            ],
        )
        .expect_err("second block fails");
        assert!(matches!(err, ToolsError::Edit(_)));
        assert!(err.to_string().contains("block 2"), "{err}");
    }

    #[test]
    fn identical_replacement_is_detected() -> Result<(), ToolsError> {
        let content = "fn stable() {}";
        let updated = apply_blocks(
            content,
            &[SearchReplaceBlock {
                search: "fn stable() {}".into(),
                replace: "fn stable() {}".into(),
            }],
        )?;
        assert_eq!(updated, content);
        Ok(())
    }

    #[tokio::test]
    async fn tool_applies_blocks_atomically_to_file() -> Result<(), ToolsError> {
        let dir = TempDir(temp_dir("edit-tool"));
        let file = dir.0.join("sample.rs");
        fs::create_dir_all(&dir.0).map_err(|source| ToolsError::io(&dir.0, source))?;
        fs::write(&file, "fn old() {}\nfn main() {}")
            .map_err(|source| ToolsError::io(&file, source))?;

        let mut registry = ToolRegistry::new();
        registry.register(EditTool);
        let result = registry
            .call(
                "edit",
                json!({
                    "file": file,
                    "response": "See below:\n<<<< SEARCH\nfn old() {}\n====\nfn new() {}\n>>>> REPLACE"
                }),
            )
            .await?;
        assert_eq!(result["blocks_applied"], 1);
        assert_eq!(result["changed"], true);

        let after = fs::read_to_string(&file).map_err(|source| ToolsError::io(&file, source))?;
        assert_eq!(after, "fn new() {}\nfn main() {}");
        Ok(())
    }

    #[tokio::test]
    async fn tool_failure_leaves_file_unchanged() -> Result<(), ToolsError> {
        let dir = TempDir(temp_dir("edit-tool-fail"));
        let file = dir.0.join("sample.rs");
        let original = "fn only() {}\n";
        fs::create_dir_all(&dir.0).map_err(|source| ToolsError::io(&dir.0, source))?;
        fs::write(&file, original).map_err(|source| ToolsError::io(&file, source))?;

        let mut registry = ToolRegistry::new();
        registry.register(EditTool);
        let err = registry
            .call(
                "edit",
                json!({
                    "file": file,
                    "response": "<<<< SEARCH\nfn ghost() {}\n====\nfn new() {}\n>>>> REPLACE"
                }),
            )
            .await
            .expect_err("must fail");
        assert!(matches!(err, ToolsError::Edit(_)));

        let after = fs::read_to_string(&file).map_err(|source| ToolsError::io(&file, source))?;
        assert_eq!(after, original);
        Ok(())
    }

    #[tokio::test]
    async fn tool_rejects_missing_blocks_and_unknown_args() {
        let mut registry = ToolRegistry::new();
        registry.register(EditTool);

        let no_blocks = registry
            .call("edit", json!({ "file": "/tmp/x", "response": "hi" }))
            .await;
        assert!(matches!(no_blocks, Err(ToolsError::InvalidArgs(_))));

        let unknown = registry.call("edit", json!({ "file": "/tmp/x" })).await;
        assert!(matches!(unknown, Err(ToolsError::InvalidArgs(_))));
    }
}
