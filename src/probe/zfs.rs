use serde::Serialize;
use std::path::Path;

/// Pool state as `zpool` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolHealth {
    /// `ONLINE`: every device is healthy.
    Online,
    /// `DEGRADED`: the pool works with reduced redundancy.
    Degraded,
    /// `FAULTED`: the pool cannot serve data.
    Faulted,
    /// `OFFLINE`: taken offline by an administrator.
    Offline,
    /// `UNAVAIL`: devices cannot be opened.
    Unavail,
    /// `REMOVED`: devices were physically removed.
    Removed,
    /// `SUSPENDED`: I/O is suspended after a failure.
    Suspended,
    /// A state this crate does not know, verbatim. Not a healthy pool.
    Unrecognized(String),
}

impl PoolHealth {
    fn parse(text: &str) -> Self {
        match text {
            "ONLINE" => Self::Online,
            "DEGRADED" => Self::Degraded,
            "FAULTED" => Self::Faulted,
            "OFFLINE" => Self::Offline,
            "UNAVAIL" => Self::Unavail,
            "REMOVED" => Self::Removed,
            "SUSPENDED" => Self::Suspended,
            other => Self::Unrecognized(other.to_owned()),
        }
    }
}

/// One zpool as `zpool list` saw it. Plain data: thresholds are the caller's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ZfsPool {
    /// Pool name.
    pub name: String,
    /// Total pool size in bytes, including redundancy overhead (raw capacity).
    pub size_bytes: u64,
    /// Bytes allocated.
    pub allocated_bytes: u64,
    /// Bytes free in the pool. Datasets may see less (reservations, slop).
    pub free_bytes: u64,
    /// The `capacity` property: whole percent of the pool in use.
    pub capacity_percent: u8,
    /// Pool state.
    pub health: PoolHealth,
    /// The pool's `readonly` property; not proof a given dataset is writable.
    pub read_only: bool,
}

impl ZfsPool {
    /// Free space as a percentage of the pool size.
    ///
    /// None when the pool reports no size.
    #[must_use]
    pub fn available_percent(&self) -> Option<f64> {
        super::storage::available_percent(self.free_bytes, self.size_bytes)
    }

    /// Whether the pool is `ONLINE`. Any other state, including an unrecognized
    /// one, is false.
    #[must_use]
    pub const fn is_online(&self) -> bool {
        matches!(self.health, PoolHealth::Online)
    }
}

/// Result of a pool probe. A failure to probe is not an empty or healthy pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolObservation {
    /// The pool was read.
    Found(ZfsPool),
    /// `zpool` ran and no such pool is imported.
    NotFound,
    /// The probe could not answer: not FreeBSD, no `zpool`, a timeout, a path
    /// off ZFS, or output that did not parse. The reason is for humans.
    Unavailable(String),
}

impl PoolObservation {
    /// The pool, when it was read.
    #[must_use]
    pub const fn pool(&self) -> Option<&ZfsPool> {
        match self {
            Self::Found(pool) => Some(pool),
            Self::NotFound | Self::Unavailable(_) => None,
        }
    }
}

/// Read one imported pool by name with `/sbin/zpool list -Hp`.
///
/// Runs without a shell, in the C locale, with a bounded deadline; the child is
/// killed on timeout. Only FreeBSD is implemented: elsewhere the answer is
/// [`PoolObservation::Unavailable`].
#[must_use]
pub fn inspect_pool(name: &str) -> PoolObservation {
    if let Err(reason) = valid_pool_name(name) {
        return PoolObservation::Unavailable(reason.to_owned());
    }
    query(name)
}

/// Read the pool backing an absolute path.
///
/// Resolves symlinks like `inspect_storage`, takes the filesystem's source
/// dataset and probes its pool. A path that is not on ZFS is
/// [`PoolObservation::Unavailable`]. Only FreeBSD is implemented.
#[must_use]
pub fn inspect_pool_for_path(path: &Path) -> PoolObservation {
    backing_pool(path).map_or_else(PoolObservation::Unavailable, |pool| inspect_pool(&pool))
}

fn valid_pool_name(name: &str) -> Result<(), &'static str> {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => {}
        _ => return Err("pool names start with a letter"),
    }
    if chars.all(|c| c.is_ascii_graphic()) {
        Ok(())
    } else {
        Err("pool names are printable ASCII without spaces")
    }
}

#[cfg(not(target_os = "freebsd"))]
fn query(_name: &str) -> PoolObservation {
    PoolObservation::Unavailable("the ZFS pool probe is only implemented on FreeBSD".into())
}

#[cfg(target_os = "freebsd")]
fn query(name: &str) -> PoolObservation {
    use crate::probe::command::{Error, run};
    use std::ffi::OsStr;
    use std::time::Duration;

    let args = [
        OsStr::new("list"),
        OsStr::new("-Hp"),
        OsStr::new("-o"),
        OsStr::new(COLUMNS),
        OsStr::new(name),
    ];
    match run("/sbin/zpool", &args, Duration::from_secs(5)) {
        Ok(output) if output.success => parse_pool(name, &String::from_utf8_lossy(&output.stdout)),
        Ok(output) => failure(&String::from_utf8_lossy(&output.stderr)),
        Err(Error::Unavailable(reason)) => PoolObservation::Unavailable(reason),
        Err(Error::PermissionDenied) => PoolObservation::Unavailable("permission denied".into()),
        Err(Error::TimedOut) => PoolObservation::Unavailable("zpool timed out".into()),
    }
}

#[cfg(not(target_os = "freebsd"))]
fn backing_pool(_path: &Path) -> Result<String, String> {
    Err("the ZFS pool probe is only implemented on FreeBSD".into())
}

#[cfg(target_os = "freebsd")]
fn backing_pool(path: &Path) -> Result<String, String> {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt;

    let path = super::storage::existing_path(path).map_err(|error| error.to_string())?;
    let c_path = CString::new(path.as_os_str().as_bytes()).map_err(|error| error.to_string())?;
    // SAFETY: statfs is a C output struct with integer and byte-array fields.
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: c_path is NUL-terminated and stat is a valid output pointer.
    if unsafe { libc::statfs(c_path.as_ptr(), &raw mut stat) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: the kernel NUL-terminates the fixed-size name arrays it fills in.
    let fstype = unsafe { CStr::from_ptr(stat.f_fstypename.as_ptr()) };
    if fstype.to_bytes() != b"zfs" {
        return Err("path is not on ZFS".into());
    }
    // SAFETY: as above, f_mntfromname is NUL-terminated.
    let source = unsafe { CStr::from_ptr(stat.f_mntfromname.as_ptr()) };
    source
        .to_str()
        .ok()
        .and_then(pool_of_dataset)
        .map(str::to_owned)
        .ok_or_else(|| "unreadable ZFS dataset name".into())
}

#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
const COLUMNS: &str = "name,size,allocated,free,capacity,health,readonly";

#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
fn pool_of_dataset(dataset: &str) -> Option<&str> {
    let pool = dataset.split(['/', '@']).next()?;
    (!pool.is_empty()).then_some(pool)
}

#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
fn failure(stderr: &str) -> PoolObservation {
    if stderr.contains("no such pool") {
        PoolObservation::NotFound
    } else {
        PoolObservation::Unavailable(format!("zpool failed: {}", stderr.trim()))
    }
}

#[cfg_attr(not(target_os = "freebsd"), allow(dead_code))]
fn parse_pool(requested: &str, text: &str) -> PoolObservation {
    match try_parse_pool(requested, text) {
        Ok(pool) => PoolObservation::Found(pool),
        Err(reason) => PoolObservation::Unavailable(format!("unreadable zpool output: {reason}")),
    }
}

fn try_parse_pool(requested: &str, text: &str) -> Result<ZfsPool, &'static str> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let (Some(line), None) = (lines.next(), lines.next()) else {
        return Err("expected exactly one pool");
    };
    let fields: Vec<&str> = line.split('\t').collect();
    let [name, size, allocated, free, capacity, health, ro_flag] = fields[..] else {
        return Err("expected seven columns");
    };
    if name != requested {
        return Err("a different pool answered");
    }
    let bytes = |field: &str| field.parse::<u64>().map_err(|_| "unreadable byte count");
    let size_bytes = bytes(size)?;
    let allocated_bytes = bytes(allocated)?;
    let free_bytes = bytes(free)?;
    let capacity_percent = capacity
        .trim_end_matches('%')
        .parse::<u8>()
        .ok()
        .filter(|percent| *percent <= 100)
        .ok_or("unreadable capacity")?;
    if allocated_bytes > size_bytes || free_bytes > size_bytes {
        return Err("implausible pool sizes");
    }
    let read_only = match ro_flag {
        "on" => true,
        "off" => false,
        _ => return Err("unreadable readonly property"),
    };
    Ok(ZfsPool {
        name: name.to_owned(),
        size_bytes,
        allocated_bytes,
        free_bytes,
        capacity_percent,
        health: PoolHealth::parse(health),
        read_only,
    })
}

#[cfg(test)]
mod tests;
