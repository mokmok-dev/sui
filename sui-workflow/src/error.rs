use std::{io, path::PathBuf};

use thiserror::Error;

use crate::host::Capability;

/// An infrastructure failure reported by a workflow host.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct HostError {
    message: String,
}

impl HostError {
    /// Creates a host infrastructure error.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// An error produced while compiling or running a workflow.
#[derive(Debug, Error)]
pub enum WorkflowError {
    /// The run configuration is incomplete or invalid.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
    /// The configured agent budget is outside the supported range.
    #[error("agent budget must be between 1 and 1024, got {0}")]
    InvalidBudget(usize),
    /// The workflow metadata declaration is invalid.
    #[error("invalid workflow metadata: {0}")]
    InvalidMeta(String),
    /// Rhai rejected the workflow while compiling it.
    #[error("workflow compilation failed: {0}")]
    Compile(String),
    /// Rhai failed while evaluating the workflow.
    #[error("workflow execution failed: {0}")]
    Runtime(String),
    /// An agent requested more capability than the host grants.
    #[error("requested capability `{requested}` exceeds host-granted `{granted}`")]
    CapabilityDenied {
        /// Capability requested by the agent invocation.
        requested: Capability,
        /// Maximum capability granted by the host.
        granted: Capability,
    },
    /// An agent requested an unrecognized capability mode.
    #[error("invalid capability_mode `{0}`; expected read-only, read-write, execute, or all")]
    InvalidCapabilityMode(String),
    /// The supplied journal does not match the current execution checksums.
    #[error("journal replay diverged: {0}")]
    JournalDivergence(String),
    /// A JSON value could not be represented by Rhai or vice versa.
    #[error("unsupported workflow value: {0}")]
    Value(String),
    /// JSON parsing or serialization failed.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    /// Reading or writing workflow state failed.
    #[error("I/O error at {path}: {source}")]
    Io {
        /// Path involved in the I/O operation.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: io::Error,
    },
}

impl From<io::Error> for WorkflowError {
    fn from(error: io::Error) -> Self {
        Self::io(".", error)
    }
}

impl WorkflowError {
    /// Creates an I/O error tied to a specific path.
    pub(crate) fn io(
        path: impl Into<PathBuf>,
        source: io::Error,
    ) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
