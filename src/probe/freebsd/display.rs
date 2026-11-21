use crate::probe::DisplayState;
use std::path::Path;

#[cfg(target_os = "freebsd")]
pub fn read() -> DisplayState {
    scan(Path::new("/dev/dri"), read_card)
}

fn scan(root: &Path, probe: impl Fn(&Path) -> DisplayState) -> DisplayState {
    let Ok(entries) = std::fs::read_dir(root) else {
        return DisplayState::Unknown;
    };
    let mut observed = false;
    let mut complete = true;
    for (index, entry) in entries.enumerate() {
        // Bound even an unexpected namespace, without claiming headless.
        if index >= 256 {
            return DisplayState::Unknown;
        }
        let Ok(entry) = entry else {
            complete = false;
            continue;
        };
        let name = entry.file_name();
        let Some(number) = name.to_str().and_then(|name| name.strip_prefix("card")) else {
            continue;
        };
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        match probe(&entry.path()) {
            DisplayState::Connected => return DisplayState::Connected,
            DisplayState::NoneDetected => observed = true,
            DisplayState::Unknown => complete = false,
        }
    }
    if observed && complete {
        DisplayState::NoneDetected
    } else {
        DisplayState::Unknown
    }
}

#[cfg(target_os = "freebsd")]
fn read_card(path: &Path) -> DisplayState {
    use super::abi;
    use std::os::fd::AsRawFd;

    let Ok(file) = std::fs::File::open(path) else {
        return DisplayState::Unknown;
    };
    let fd = file.as_raw_fd();
    query_card(
        // SAFETY: query_card establishes each ABI structure's output capacities
        // and keeps its embedded arrays alive for the synchronous calls.
        |resources| unsafe { abi::query(fd, abi::CARD_RESOURCES, resources) },
        // SAFETY: as above, for the connector's single mode slot.
        |connector| unsafe { abi::query(fd, abi::CONNECTOR, connector) },
    )
}

#[cfg(target_os = "freebsd")]
fn query_card(
    mut resources_query: impl FnMut(&mut super::abi::CardResources) -> std::io::Result<()>,
    mut connector_query: impl FnMut(&mut super::abi::Connector) -> std::io::Result<()>,
) -> DisplayState {
    use super::abi;

    // Hotplug can change array sizes between queries; bound both allocation
    // and retries. Never use a truncated or unfilled connector array.
    for _ in 0..3 {
        let mut resources = abi::CardResources::default();
        // The zero-capacity query returns counts, not arrays.
        if resources_query(&mut resources).is_err() {
            return DisplayState::Unknown;
        }
        let Some(capacity) = usize::try_from(resources.count_connectors)
            .ok()
            .filter(|count| (1..=4096).contains(count))
        else {
            return DisplayState::Unknown;
        };
        let mut ids = vec![0_u32; capacity];
        resources.connector_ids = address(ids.as_mut_ptr());
        resources.count_fbs = 0;
        resources.count_crtcs = 0;
        resources.count_encoders = 0;
        // ids has count_connectors writable entries; every other output array
        // keeps a zero capacity and a null pointer.
        if resources_query(&mut resources).is_err() {
            return DisplayState::Unknown;
        }
        let Some(filled) = usize::try_from(resources.count_connectors)
            .ok()
            .filter(|count| *count <= ids.len())
        else {
            continue;
        };
        ids.truncate(filled);
        return connections(ids.iter().map(|&id| {
            // One mode slot avoids DRM's count_modes=0 forced hardware probe
            // when this reader happens to be DRM master. No modesetting.
            let mut mode = abi::ModeInfo::default();
            let mut connector = abi::Connector {
                connector_id: id,
                count_modes: 1,
                modes: address(mode.as_mut_ptr()),
                ..Default::default()
            };
            connector_query(&mut connector)
                .ok()
                .map(|()| connector.connection)
        }));
    }
    DisplayState::Unknown
}

#[cfg(target_os = "freebsd")]
fn address(pointer: *mut u32) -> u64 {
    // The kernel writes through this address, so its provenance is exposed.
    pointer.expose_provenance() as u64
}

fn connections(values: impl IntoIterator<Item = Option<u32>>) -> DisplayState {
    let mut observed = false;
    let mut complete = true;
    for value in values {
        match value {
            Some(1) => return DisplayState::Connected, // DRM_MODE_CONNECTED
            Some(2) => observed = true,                // DRM_MODE_DISCONNECTED
            _ => complete = false,
        }
    }
    if observed && complete {
        DisplayState::NoneDetected
    } else {
        DisplayState::Unknown
    }
}

#[cfg(test)]
mod tests;
