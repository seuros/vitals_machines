//! Explicit, read-only indexing observations. No preferences or services change.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[cfg(any(target_os = "freebsd", test))]
mod freebsd;
#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(any(target_os = "macos", test))]
mod macos;
mod process;

/// A failed or unsupported probe is never a successful empty measurement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum IndexingObservation<T> {
    /// The probe ran and produced a value.
    Available {
        /// What the probe observed.
        value: T,
    },
    /// The probe failed or its output could not be understood.
    Unavailable {
        /// Why, for humans; not a stable format.
        reason: String,
    },
    /// The OS refused to run or answer the probe.
    PermissionDenied,
    /// The probe missed its one-second deadline and was killed.
    TimedOut,
    /// This platform has no such probe; nothing was run.
    Unsupported,
}

/// Spotlight indexing on the workspace volume, as `mdutil` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotlightState {
    /// Indexing is enabled.
    Enabled,
    /// Indexing (or indexing and searching) is disabled.
    Disabled,
}

/// One running process recognized as a file indexer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct IndexerProcess {
    /// Process id in the caller's PID namespace.
    pub pid: u32,
    /// Executable basename, not a command line or service label.
    pub name: String,
    /// `ps` CPU average: decaying (up to a minute) on macOS and FreeBSD,
    /// process-lifetime on Linux. Not instantaneous, per-workspace, or throughput.
    pub cpu_percent: f64,
}

/// Result of one [`inspect_indexing`] call.
#[derive(Debug, Clone, Serialize)]
pub struct IndexingSnapshot {
    /// Wall-clock time taken before any probe ran.
    pub observed_at: SystemTime,
    /// The workspace path as the caller passed it.
    pub workspace: PathBuf,
    /// macOS mount point resolved from the workspace, not an assumed `/`.
    /// Not collected on Linux or FreeBSD.
    pub volume: Option<PathBuf>,
    /// Spotlight state for `volume`; `Unsupported` off macOS.
    pub spotlight: IndexingObservation<SpotlightState>,
    /// Visible host processes, not just processes indexing this workspace.
    pub processes: IndexingObservation<Vec<IndexerProcess>>,
}

/// Inspect visible indexers, plus workspace-volume Spotlight state on macOS.
///
/// Linux and FreeBSD report known indexer processes, not a universal indexing
/// enabled state.
/// Each subprocess has a one-second deadline and bounded output. Filesystem
/// resolution is synchronous I/O; use a bounded blocking worker in async callers.
#[must_use]
pub fn inspect_indexing(workspace: &Path) -> IndexingSnapshot {
    let snapshot = IndexingSnapshot {
        observed_at: SystemTime::now(),
        workspace: workspace.to_path_buf(),
        volume: None,
        spotlight: IndexingObservation::Unsupported,
        processes: IndexingObservation::Unsupported,
    };
    #[cfg(target_os = "macos")]
    {
        macos::inspect(snapshot)
    }
    #[cfg(target_os = "linux")]
    {
        linux::inspect(snapshot)
    }
    #[cfg(target_os = "freebsd")]
    {
        freebsd::inspect(snapshot)
    }
}

#[cfg(test)]
mod tests;
