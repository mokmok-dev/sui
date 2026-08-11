//! Exclusive run lease for a checkpoint path (single-writer fence).
//!
//! celld fences each cell with an ownership epoch so two writers never share
//! one durable store. A workflow checkpoint is the analogous durable store:
//! concurrent `Engine::run` processes on the same path must not interleave
//! atomic journal writes.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use crate::WorkflowError;

/// An exclusive lease held for the duration of one checkpointed run.
#[derive(Debug)]
pub struct RunLease {
    path: PathBuf,
    _file: File,
}

impl RunLease {
    /// Acquires an exclusive lock beside `checkpoint`.
    ///
    /// Stale locks from dead processes are reclaimed. A live holder yields
    /// [`WorkflowError::LeaseHeld`].
    ///
    /// # Errors
    ///
    /// Returns I/O errors or [`WorkflowError::LeaseHeld`].
    pub fn acquire(checkpoint: &Path) -> Result<Self, WorkflowError> {
        let path = lock_path(checkpoint);
        if let Some(holder_pid) = read_holder_pid(&path)? {
            if pid_is_alive(holder_pid) {
                return Err(WorkflowError::LeaseHeld {
                    path: path.clone(),
                    holder_pid: Some(holder_pid),
                });
            }
            let _ = fs::remove_file(&path);
        }

        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let pid = std::process::id();
                write!(file, "{pid}").map_err(|error| WorkflowError::io(&path, error))?;
                file.sync_all()
                    .map_err(|error| WorkflowError::io(&path, error))?;
                Ok(Self { path, _file: file })
            },
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let holder_pid = read_holder_pid(&path)?;
                Err(WorkflowError::LeaseHeld { path, holder_pid })
            },
            Err(error) => Err(WorkflowError::io(&path, error)),
        }
    }

    /// Returns the lock file path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for RunLease {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn lock_path(checkpoint: &Path) -> PathBuf {
    let mut path = checkpoint.as_os_str().to_owned();
    path.push(".runlock");
    PathBuf::from(path)
}

fn read_holder_pid(path: &Path) -> Result<Option<u32>, WorkflowError> {
    match fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(WorkflowError::io(path, error)),
        Ok(_) => {
            let mut file = File::open(path).map_err(|error| WorkflowError::io(path, error))?;
            let mut contents = String::new();
            file.read_to_string(&mut contents)
                .map_err(|error| WorkflowError::io(path, error))?;
            Ok(contents.trim().parse::<u32>().ok())
        },
    }
}

fn pid_is_alive(pid: u32) -> bool {
    pid != 0 && Path::new(&format!("/proc/{pid}")).exists()
}
