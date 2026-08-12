//! Agent tool-calling foundation for deep research.
//!
//! Provides:
//! - [`bm25`]: Okapi BM25 indexing/search over local code documents
//! - [`tantivy_index`]: persistent Tantivy lexical backend (same corpus walk)
//! - [`corpus`]: shared tree-walk / skip policy for lexical indexers
//! - [`bash`]: async non-blocking bash session primitives (pipes, not a PTY)
//!   plus [`run_line`] for one-shot commands
//! - [`tool`]: thin [`Tool`] trait + registry with builtin `code_search` / `bash`
//!
//! # Threat model
//!
//! This crate is a **foundation**, not a sandbox. [`bash::BashSession`] /
//! [`tool::BashTool`] execute arbitrary shell commands with the host process
//! credentials. There is no filesystem jail, network policy, or environment
//! scrubbing. Callers that expose these tools to untrusted agents must isolate
//! the process (container, VM, least-privilege user) themselves.

pub mod bash;
pub mod bm25;
pub mod corpus;
pub mod error;
pub mod tantivy_index;
pub mod tool;

pub use bash::{
    BashSession, CommandOutput, DEFAULT_RUN_TIMEOUT, MAX_BUFFER_BYTES, ProcessState, SessionOutput,
    WaitOutcome, run_line,
};
pub use bm25::{
    Bm25Index, DEFAULT_B, DEFAULT_K1, LexicalSearch, MAX_FILE_BYTES, MAX_INDEX_DOCS, SNIPPET_CHARS,
    SearchHit, tokenize,
};
pub use error::ToolsError;
pub use tantivy_index::TantivyIndex;
pub use tool::{
    BashTool, CodeSearchTool, MAX_SEARCH_LIMIT, Tool, ToolFuture, ToolRegistry, builtin_registry,
    code_search_registry,
};
