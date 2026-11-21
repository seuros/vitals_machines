use crate::probe::FreebsdAcpiDiagnostics;
use crate::probe::sysctl;
use std::collections::BTreeMap;
use std::ffi::CStr;

pub fn read() -> FreebsdAcpiDiagnostics {
    // Called only after detect() establishes an unjailed caller. Enumerate
    // names, not arbitrary values: only the allowlisted leaves are queried.
    FreebsdAcpiDiagnostics {
        driver_lid_states: collect(c"dev.acpi_lid", lid_leaf),
        driver_video_activity: collect(c"hw.acpi.video", video_leaf),
    }
}

fn collect(root: &CStr, accepts: impl Fn(&str) -> bool) -> Option<BTreeMap<String, Option<i32>>> {
    let mut values = BTreeMap::new();
    for name in sysctl::names(root, 1024)? {
        let text = name.to_str().ok()?;
        if accepts(text) {
            values.insert(text.to_owned(), sysctl::integer(&name));
        }
    }
    Some(values)
}

fn lid_leaf(name: &str) -> bool {
    name.strip_prefix("dev.acpi_lid.")
        .and_then(|name| name.strip_suffix(".state"))
        .is_some_and(unit_number)
}

fn video_leaf(name: &str) -> bool {
    let Some(output) = name
        .strip_prefix("hw.acpi.video.")
        .and_then(|name| name.strip_suffix(".active"))
    else {
        return false;
    };
    ["crt", "tv", "ext", "lcd", "out"]
        .iter()
        .any(|prefix| output.strip_prefix(prefix).is_some_and(unit_number))
}

fn unit_number(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}
