//! Prints what the probes see on this machine, as JSON: the machine
//! snapshot, then the storage holding each path given (the working
//! directory when none is).
//!
//! `cargo run --example snapshot --features probe -- /tmp`

use std::path::PathBuf;
use vitals_machines::probe::{StorageRole, StorageTarget, inspect_machine, inspect_storage};

fn main() {
    let paths: Vec<PathBuf> = match std::env::args()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>()
    {
        paths if paths.is_empty() => {
            vec![std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))]
        }
        paths => paths,
    };
    let targets: Vec<StorageTarget> = paths
        .into_iter()
        .map(|path| StorageTarget {
            role: StorageRole::State,
            path,
        })
        .collect();
    let snapshot = serde_json::json!({
        "machine": inspect_machine(),
        "storage": inspect_storage(&targets),
    });
    match serde_json::to_string_pretty(&snapshot) {
        Ok(json) => println!("{json}"),
        Err(error) => eprintln!("cannot encode the snapshot: {error}"),
    }
}
