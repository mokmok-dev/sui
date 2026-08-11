//! Okapi BM25 sparse retrieval for local code corpora.
//!
//! Inspired by Agent+BM25 ranked discovery (arXiv:2607.26497): lexical
//! retrieval over a codebase beats blind filesystem exploration at scale.
//!
//! # Tokenization
//!
//! Tokens are **ASCII alphanumeric / `_` only** (plus camelCase / `snake_case`
//! splitting). Non-ASCII identifiers are not split into Unicode word breaks;
//! a Unicode-aware tokenizer is deferred as out of scope for this foundation.
//!
//! # Symlink policy
//!
//! Directory walks **do not follow symlinks** (`file_type` from `DirEntry` is
//! used; symlink-to-dir and symlink-to-file entries are skipped). This avoids
//! escaping the indexed tree via symlink roots. Callers that need symlink
//! follow should canonicalize externally and pass a real path.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Read,
    path::Path,
};

use crate::ToolsError;

/// Default Okapi BM25 term-frequency saturation parameter.
pub const DEFAULT_K1: f64 = 1.2;
/// Default Okapi BM25 length-normalization parameter.
pub const DEFAULT_B: f64 = 0.75;

/// Maximum bytes read from a single source file during [`Bm25Index::index_tree`].
pub const MAX_FILE_BYTES: u64 = 1_048_576; // 1 MiB
/// Maximum number of documents retained in one index built by [`Bm25Index::index_tree`].
pub const MAX_INDEX_DOCS: usize = 10_000;
/// Preview length (chars) stored on each [`SearchHit`].
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

/// A ranked search hit.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SearchHit {
    /// Stable document identifier (often the path).
    pub doc_id: String,
    /// Source path associated with the document, if any.
    pub path: String,
    /// BM25 score (higher is better).
    pub score: f64,
    /// Short text preview for agent context (first [`SNIPPET_CHARS`] chars).
    pub snippet: String,
}

/// In-memory Okapi BM25 index over `(path, text)` documents.
#[derive(Debug, Clone)]
pub struct Bm25Index {
    k1: f64,
    b: f64,
    docs: Vec<Document>,
    /// term -> (`doc_index` -> term frequency)
    postings: HashMap<String, HashMap<usize, u32>>,
    total_len: usize,
}

#[derive(Debug, Clone)]
struct Document {
    id: String,
    path: String,
    len: usize,
    snippet: String,
}

impl Default for Bm25Index {
    fn default() -> Self {
        Self::new(DEFAULT_K1, DEFAULT_B)
    }
}

impl Bm25Index {
    /// Creates an empty index with the given BM25 parameters.
    #[must_use]
    pub fn new(
        k1: f64,
        b: f64,
    ) -> Self {
        Self {
            k1,
            b,
            docs: Vec::new(),
            postings: HashMap::new(),
            total_len: 0,
        }
    }

    /// Number of indexed documents.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.docs.len()
    }

    /// Returns `true` when no documents are indexed.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.docs.is_empty()
    }

    /// Indexes a document. `doc_id` should be unique; duplicates append a new
    /// document rather than replacing (callers should rebuild if needed).
    pub fn add_document(
        &mut self,
        doc_id: impl Into<String>,
        path: impl Into<String>,
        text: &str,
    ) {
        let tokens = tokenize(text);
        let doc_index = self.docs.len();
        let len = tokens.len();
        self.docs.push(Document {
            id: doc_id.into(),
            path: path.into(),
            len,
            snippet: make_snippet(text),
        });
        self.total_len = self.total_len.saturating_add(len);

        let mut tf: HashMap<String, u32> = HashMap::new();
        for token in tokens {
            *tf.entry(token).or_insert(0) += 1;
        }

        for (term, freq) in tf {
            self.postings
                .entry(term)
                .or_default()
                .insert(doc_index, freq);
        }
    }

    /// Indexes UTF-8 text files under `root` whose extension is in `extensions`
    /// (compared without a leading dot, case-insensitive).
    ///
    /// **Skips** (does not fail the walk):
    /// - unreadable files / non-UTF-8 contents
    /// - files larger than [`MAX_FILE_BYTES`]
    /// - likely secret filenames (`.env`, `.env.*`, `*.pem`, …)
    /// - directories in [`SKIP_DIRS`], and any directory whose name starts with `.`
    /// - symlinks (see module symlink policy)
    ///
    /// Stops adding documents once [`MAX_INDEX_DOCS`] is reached.
    ///
    /// # Errors
    ///
    /// Returns [`ToolsError::Io`] only when the root directory itself cannot be
    /// read. Per-file failures are skipped.
    pub fn index_tree(
        root: &Path,
        extensions: &[&str],
    ) -> Result<Self, ToolsError> {
        let mut index = Self::default();
        let ext_set: HashSet<String> = extensions
            .iter()
            .map(|ext| ext.trim_start_matches('.').to_ascii_lowercase())
            .collect();
        walk_files(root, &mut |path| {
            if index.len() >= MAX_INDEX_DOCS {
                return Ok(WalkControl::Stop);
            }
            let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
                return Ok(WalkControl::Continue);
            };
            if !ext_set.contains(&ext.to_ascii_lowercase()) {
                return Ok(WalkControl::Continue);
            }
            if is_secret_path(path) {
                return Ok(WalkControl::Continue);
            }
            // Bound the read to avoid TOCTOU growth past metadata.len().
            let Ok(file) = fs::File::open(path) else {
                return Ok(WalkControl::Continue);
            };
            let mut limited = file.take(MAX_FILE_BYTES.saturating_add(1));
            let mut bytes = Vec::new();
            if limited.read_to_end(&mut bytes).is_err() {
                return Ok(WalkControl::Continue);
            }
            if bytes.len() as u64 > MAX_FILE_BYTES {
                return Ok(WalkControl::Continue);
            }
            let Ok(text) = String::from_utf8(bytes) else {
                return Ok(WalkControl::Continue);
            };
            let path_str = path.to_string_lossy().into_owned();
            index.add_document(path_str.clone(), path_str, &text);
            Ok(WalkControl::Continue)
        })?;
        Ok(index)
    }

    /// Searches the index and returns up to `limit` hits, highest score first.
    #[must_use]
    #[allow(clippy::cast_precision_loss)] // BM25 scores are inherently f64 approximations.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Vec<SearchHit> {
        if self.docs.is_empty() || limit == 0 {
            return Vec::new();
        }

        let query_terms = unique_tokens(query);
        if query_terms.is_empty() {
            return Vec::new();
        }

        let n = self.docs.len() as f64;
        let avgdl = self.total_len as f64 / n;

        let mut scores: HashMap<usize, f64> = HashMap::new();
        for term in &query_terms {
            let Some(posting) = self.postings.get(term) else {
                continue;
            };
            let df = posting.len() as f64;
            let idf = idf(n, df);
            for (&doc_idx, &tf) in posting {
                let doc_len = self.docs[doc_idx].len as f64;
                let tf = f64::from(tf);
                let denom = self.k1.mul_add(
                    self.b
                        .mul_add(doc_len / avgdl.max(f64::EPSILON), 1.0 - self.b),
                    tf,
                );
                let term_score = idf * (tf * (self.k1 + 1.0)) / denom;
                *scores.entry(doc_idx).or_insert(0.0) += term_score;
            }
        }

        let mut hits: Vec<SearchHit> = scores
            .into_iter()
            .map(|(doc_idx, score)| {
                let doc = &self.docs[doc_idx];
                SearchHit {
                    doc_id: doc.id.clone(),
                    path: doc.path.clone(),
                    score,
                    snippet: doc.snippet.clone(),
                }
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .map_or(std::cmp::Ordering::Equal, |ordering| ordering)
                .then_with(|| a.path.cmp(&b.path))
        });
        hits.truncate(limit);
        hits
    }
}

enum WalkControl {
    Continue,
    Stop,
}

/// Lucene-style BM25 IDF: `ln(1 + (N - df + 0.5) / (df + 0.5))`.
fn idf(
    n: f64,
    df: f64,
) -> f64 {
    ((n - df + 0.5) / (df + 0.5)).ln_1p()
}

fn make_snippet(text: &str) -> String {
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

fn is_secret_path(path: &Path) -> bool {
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

/// Code-aware tokenization: split on non-alphanumeric boundaries, then split
/// `camelCase` / `snake_case` identifiers into lowercase subtokens.
///
/// ASCII-only; see module docs.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch);
        } else if !current.is_empty() {
            split_identifier(&current, &mut tokens);
            current.clear();
        }
    }
    if !current.is_empty() {
        split_identifier(&current, &mut tokens);
    }
    tokens
}

fn unique_tokens(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for token in tokenize(text) {
        if seen.insert(token.clone()) {
            out.push(token);
        }
    }
    out
}

fn split_identifier(
    ident: &str,
    out: &mut Vec<String>,
) {
    for part in ident.split('_').filter(|part| !part.is_empty()) {
        let chars: Vec<char> = part.chars().collect();
        let mut start = 0;
        for i in 1..chars.len() {
            let prev = chars[i - 1];
            let cur = chars[i];
            let next_lower = chars.get(i + 1).is_some_and(char::is_ascii_lowercase);
            let boundary = (prev.is_ascii_lowercase() && cur.is_ascii_uppercase())
                || (prev.is_ascii_uppercase() && cur.is_ascii_uppercase() && next_lower);
            if boundary {
                push_lower_token(&chars[start..i], out);
                start = i;
            }
        }
        push_lower_token(&chars[start..], out);
    }
}

fn push_lower_token(
    chars: &[char],
    out: &mut Vec<String>,
) {
    if chars.is_empty() {
        return;
    }
    let mut token = String::with_capacity(chars.len());
    for ch in chars {
        token.push(ch.to_ascii_lowercase());
    }
    out.push(token);
}

fn walk_files(
    root: &Path,
    visit: &mut dyn FnMut(&Path) -> Result<WalkControl, ToolsError>,
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
                    WalkControl::Continue => {},
                    WalkControl::Stop => return Ok(()),
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "sui-tools-bm25-{label}-{}-{nanos}-{seq}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn tokenize_splits_camel_and_snake() {
        let tokens = tokenize("parseHttpResponse snake_case_value XMLParser");
        assert!(tokens.contains(&"parse".into()));
        assert!(tokens.contains(&"http".into()));
        assert!(tokens.contains(&"response".into()));
        assert!(tokens.contains(&"snake".into()));
        assert!(tokens.contains(&"case".into()));
        assert!(tokens.contains(&"value".into()));
        assert!(tokens.contains(&"xml".into()));
        assert!(tokens.contains(&"parser".into()));
    }

    #[test]
    fn ranks_relevant_document_highest() {
        let mut index = Bm25Index::default();
        index.add_document(
            "a",
            "auth.rs",
            "fn authenticate_user(password: &str) { verify_password(password) }",
        );
        index.add_document(
            "b",
            "render.rs",
            "fn render_widget(layout: Layout) { draw_frame(layout) }",
        );
        index.add_document(
            "c",
            "password.rs",
            "fn hash_password(password: &str) -> Digest { blake3(password) }",
        );

        let hits = index.search("password authenticate", 3);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].path, "auth.rs");
        assert_eq!(hits[1].path, "password.rs");
        assert!(hits[0].score.is_finite());
        assert!(hits[1].score.is_finite());
        assert!(hits[0].score > hits[1].score);
        assert!(!hits[0].snippet.is_empty());
    }

    #[test]
    fn punctuation_query_and_ext_normalization() -> Result<(), ToolsError> {
        let dir = TempDir(temp_dir("punct"));
        fs::create_dir_all(dir.0.join("src")).map_err(|source| ToolsError::io(&dir.0, source))?;
        fs::write(dir.0.join("src/lib.rs"), "fn find_widget() {}")
            .map_err(|source| ToolsError::io(dir.0.join("src/lib.rs"), source))?;

        let index = Bm25Index::index_tree(&dir.0, &[".RS", "rs"])?;
        let hits = index.search("find-widget!!!", 5);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("lib.rs"));
        Ok(())
    }

    #[test]
    fn empty_query_or_index_returns_no_hits() {
        let index = Bm25Index::default();
        assert!(index.search("anything", 10).is_empty());

        let mut index = Bm25Index::default();
        index.add_document("a", "a.rs", "hello world");
        assert!(index.search("", 10).is_empty());
        assert!(index.search("hello", 0).is_empty());
    }

    #[test]
    fn index_tree_indexes_matching_extensions() -> Result<(), ToolsError> {
        let dir = TempDir(temp_dir("index"));
        fs::create_dir_all(dir.0.join("src")).map_err(|source| ToolsError::io(&dir.0, source))?;
        fs::write(dir.0.join("src/lib.rs"), "fn search_code() {}")
            .map_err(|source| ToolsError::io(dir.0.join("src/lib.rs"), source))?;
        fs::write(dir.0.join("src/notes.txt"), "search_code mention")
            .map_err(|source| ToolsError::io(dir.0.join("src/notes.txt"), source))?;

        let index = Bm25Index::index_tree(&dir.0, &["rs"])?;
        assert_eq!(index.len(), 1);
        let hits = index.search("search_code", 5);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].path.ends_with("lib.rs"));
        Ok(())
    }

    #[test]
    fn index_tree_skips_dependency_and_dot_dirs() -> Result<(), ToolsError> {
        let dir = TempDir(temp_dir("skip-dirs"));
        for sub in [
            "src",
            "target/debug",
            ".git",
            "node_modules/pkg",
            "vendor/lib",
            "__pycache__",
            "dist",
        ] {
            fs::create_dir_all(dir.0.join(sub)).map_err(|source| ToolsError::io(&dir.0, source))?;
        }
        fs::write(dir.0.join("src/lib.rs"), "fn keep_me() {}")
            .map_err(|source| ToolsError::io(dir.0.join("src/lib.rs"), source))?;
        fs::write(dir.0.join("target/debug/skip.rs"), "fn skip_target() {}")
            .map_err(|source| ToolsError::io(dir.0.join("target/debug/skip.rs"), source))?;
        fs::write(dir.0.join(".git/config.rs"), "fn skip_git() {}")
            .map_err(|source| ToolsError::io(dir.0.join(".git/config.rs"), source))?;
        fs::write(dir.0.join("node_modules/pkg/x.rs"), "fn skip_nm() {}")
            .map_err(|source| ToolsError::io(dir.0.join("node_modules/pkg/x.rs"), source))?;
        fs::write(dir.0.join("vendor/lib/x.rs"), "fn skip_vendor() {}")
            .map_err(|source| ToolsError::io(dir.0.join("vendor/lib/x.rs"), source))?;
        fs::write(dir.0.join("__pycache__/x.rs"), "fn skip_pyc() {}")
            .map_err(|source| ToolsError::io(dir.0.join("__pycache__/x.rs"), source))?;
        fs::write(dir.0.join("dist/x.rs"), "fn skip_dist() {}")
            .map_err(|source| ToolsError::io(dir.0.join("dist/x.rs"), source))?;

        let index = Bm25Index::index_tree(&dir.0, &["rs"])?;
        assert_eq!(index.len(), 1);
        assert_eq!(index.search("keep_me", 5).len(), 1);
        assert!(index.search("skip_target", 5).is_empty());
        assert!(index.search("skip_git", 5).is_empty());
        assert!(index.search("skip_nm", 5).is_empty());
        assert!(index.search("skip_vendor", 5).is_empty());
        assert!(index.search("skip_pyc", 5).is_empty());
        assert!(index.search("skip_dist", 5).is_empty());
        Ok(())
    }

    #[test]
    fn index_tree_skips_bad_utf8() -> Result<(), ToolsError> {
        let dir = TempDir(temp_dir("bad-utf8"));
        fs::create_dir_all(dir.0.join("src")).map_err(|source| ToolsError::io(&dir.0, source))?;
        fs::write(dir.0.join("src/good.rs"), "fn good_utf8() {}")
            .map_err(|source| ToolsError::io(dir.0.join("src/good.rs"), source))?;
        fs::write(dir.0.join("src/bad.rs"), [0xff, 0xfe, 0xfd])
            .map_err(|source| ToolsError::io(dir.0.join("src/bad.rs"), source))?;

        let index = Bm25Index::index_tree(&dir.0, &["rs"])?;
        assert_eq!(index.len(), 1);
        assert_eq!(index.search("good_utf8", 5).len(), 1);
        Ok(())
    }

    #[test]
    fn index_tree_skips_huge_and_env_files() -> Result<(), ToolsError> {
        let dir = TempDir(temp_dir("huge-env"));
        fs::create_dir_all(dir.0.join("src")).map_err(|source| ToolsError::io(&dir.0, source))?;
        fs::write(dir.0.join("src/keep.rs"), "fn keep_token_xyz() {}")
            .map_err(|source| ToolsError::io(dir.0.join("src/keep.rs"), source))?;

        let huge_len = usize::try_from(MAX_FILE_BYTES).expect("MAX_FILE_BYTES fits usize") + 8;
        let huge = vec![b'a'; huge_len];
        fs::write(dir.0.join("src/huge.rs"), &huge)
            .map_err(|source| ToolsError::io(dir.0.join("src/huge.rs"), source))?;

        // Extension would match (`local` / `staging`); name policy must still skip.
        fs::write(
            dir.0.join(".env.local"),
            "fn zz_envlocal_unique_marker() {}",
        )
        .map_err(|source| ToolsError::io(dir.0.join(".env.local"), source))?;
        fs::write(
            dir.0.join(".env.staging"),
            "fn zz_envstaging_unique_marker() {}",
        )
        .map_err(|source| ToolsError::io(dir.0.join(".env.staging"), source))?;

        let index = Bm25Index::index_tree(&dir.0, &["rs", "local", "staging"])?;
        assert_eq!(index.search("keep_token_xyz", 5).len(), 1);
        assert!(index.search("zz_envlocal_unique_marker", 5).is_empty());
        assert!(index.search("zz_envstaging_unique_marker", 5).is_empty());
        assert_eq!(index.len(), 1, "huge + env files must be skipped");
        Ok(())
    }

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
