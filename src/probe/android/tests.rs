use super::*;
use crate::probe::{BatteryState, PowerSource};

#[test]
fn cpu_zones_by_their_android_names() {
    // Galaxy Z Fold6 (Snapdragon 8 Gen 3), Galaxy Tab S7 (Snapdragon 865)
    // and Pixel 6 Pro (Tensor), as listed on the devices.
    for cpu in [
        "cpu-1-0-0",
        "cpu-0-1-0",
        "cpuss-0",
        "cpu-0-0-usr",
        "cpu-1-7-usr",
        "BIG",
        "MID",
        "LITTLE",
        "cpu0",
        "cpu-thermal",
    ] {
        assert!(cpu_zone(cpu), "{cpu}");
    }
    for other in [
        "cpu-0-0-step",
        "apc-0-max-step",
        "gpuss-0",
        "aoss-0",
        "ddr",
        "camera-0",
        "TPU",
        "G3D",
        "nsphvx-0",
        "battery",
    ] {
        assert!(!cpu_zone(other), "{other}");
    }
}

#[test]
fn the_thermal_status_maps_coarsely() {
    assert_eq!(thermal_state(-1), ThermalState::Unknown);
    assert_eq!(thermal_state(0), ThermalState::Normal);
    assert_eq!(thermal_state(1), ThermalState::Normal);
    assert_eq!(thermal_state(2), ThermalState::Warning);
    assert_eq!(thermal_state(3), ThermalState::Warning);
    assert_eq!(thermal_state(4), ThermalState::Critical);
    assert_eq!(thermal_state(6), ThermalState::Critical);
    assert_eq!(thermal_state(7), ThermalState::Unknown);
}

#[test]
fn the_device_class_is_read_not_guessed() {
    let class = |c| form_factor(Some(c)).0;
    assert_eq!(class("phone"), FormFactor::Phone);
    assert_eq!(class("tablet"), FormFactor::Tablet);
    assert_eq!(class("tablet,nosdcard"), FormFactor::Tablet);
    assert_eq!(class("watch"), FormFactor::Other);
    // A Pixel says only this: no evidence of anything.
    assert_eq!(
        form_factor(Some("nosdcard")),
        (FormFactor::Unknown, FormFactorEvidence::Unknown)
    );
    assert_eq!(
        form_factor(None),
        (FormFactor::Unknown, FormFactorEvidence::Unknown)
    );
    assert_eq!(
        form_factor(Some("phone")).1,
        FormFactorEvidence::SystemProperty
    );
}

#[test]
fn android_zones_are_read_with_the_android_names() {
    let root = tempfile::tempdir().unwrap();
    let zone = |n: u32, name: &str, millis: &str| {
        let dir = root
            .path()
            .join(format!("sys/class/thermal/thermal_zone{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("type"), name).unwrap();
        std::fs::write(dir.join("temp"), millis).unwrap();
    };
    zone(0, "aoss-0", "29800");
    zone(1, "cpu-1-0-0", "31400");
    zone(2, "cpu-1-0-0-step", "31400");
    zone(3, "gpuss-0", "30600");
    zone(4, "cpuss-0", "30600");
    let found = zones(root.path(), cpu_zone);
    let read: Vec<(&str, Option<f64>)> = found
        .iter()
        .map(|t| (t.label.as_str(), t.celsius))
        .collect();
    assert_eq!(read, [("cpu-1-0-0", Some(31.4)), ("cpuss-0", Some(30.6))]);
}

fn supply(root: &Path, name: &str, nodes: &[(&str, &str)]) {
    crate::probe::linux::test_device(root, name, nodes).unwrap();
}

/// A supply whose nodes SELinux denies: present in the listing, unreadable.
fn denied(root: &Path, name: &str) {
    use std::os::unix::fs::PermissionsExt;
    supply(root, name, &[("type", "USB")]);
    let node = root.join(name).join("type");
    std::fs::set_permissions(node, std::fs::Permissions::from_mode(0o000)).unwrap();
}

#[test]
fn a_denied_vendor_supply_does_not_void_the_battery() {
    // Pixel 6 Pro on a charger: `maxfg` and `wireless` are denied.
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    supply(
        root,
        "battery",
        &[
            ("type", "Battery"),
            ("status", "Charging"),
            ("capacity", "6"),
        ],
    );
    supply(root, "usb", &[("type", "USB"), ("online", "1")]);
    supply(root, "gcpm", &[("type", "Unknown"), ("online", "1")]);
    denied(root, "wireless");
    let power = read_power(root);
    assert_eq!(power.source, PowerSource::External);
    assert_eq!(power.external_power, Some(true));
    let batteries = power.batteries.unwrap();
    assert_eq!(batteries.len(), 1);
    assert_eq!(batteries[0].charge_percent, Some(6));
    assert_eq!(batteries[0].state, BatteryState::Charging);
}

#[test]
fn unplugged_is_only_claimed_when_every_supply_reads() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    supply(
        root,
        "battery",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("capacity", "80"),
        ],
    );
    supply(root, "usb", &[("type", "USB"), ("online", "0")]);
    supply(root, "ac", &[("type", "Mains"), ("online", "0")]);
    assert_eq!(read_power(root).external_power, Some(false));
    assert_eq!(read_power(root).source, PowerSource::Battery);

    denied(root, "wireless");
    let power = read_power(root);
    assert_eq!(power.external_power, None);
    // The battery still says it is draining.
    assert_eq!(power.source, PowerSource::Battery);
}

#[test]
fn a_denied_supply_class_is_unknown() {
    // Galaxy Z Fold6: the directory itself cannot be listed.
    let power = read_power(Path::new("/nonexistent/power_supply"));
    assert_eq!(power.source, PowerSource::Unknown);
    assert_eq!(power.external_power, None);
    assert_eq!(power.batteries, None);
}

#[test]
fn binder_capacity_is_bounded_and_does_not_invent_power_source() {
    let mut power = PowerInfo::default();
    for value in [None, Some(-1), Some(101), Some(i64::MAX)] {
        merge_capacity(&mut power, value);
        assert_eq!(power.batteries, None);
    }
    merge_capacity(&mut power, Some(42));
    let battery = &power.batteries.as_ref().unwrap()[0];
    assert_eq!(battery.charge_percent, Some(42));
    assert_eq!(battery.present, None);
    assert_eq!(battery.state, BatteryState::Unknown);
    assert_eq!(power.source, PowerSource::Unknown);
    assert_eq!(power.external_power, None);
    merge_capacity(&mut power, Some(50));
    assert_eq!(
        power.batteries.as_ref().unwrap()[0].charge_percent,
        Some(42)
    );
}

#[test]
fn binder_does_not_override_absent_battery_or_empty_enumeration() {
    let mut power = PowerInfo {
        batteries: Some(vec![]),
        ..PowerInfo::default()
    };
    assert!(!needs_capacity(&power));
    merge_capacity(&mut power, Some(42));
    assert_eq!(power.batteries, Some(vec![]));
    power.batteries = Some(vec![crate::probe::BatteryInfo {
        name: "battery".into(),
        kind: BatteryKind::System,
        present: Some(false),
        charge_percent: None,
        state: BatteryState::Unknown,
    }]);
    assert!(!needs_capacity(&power));
    merge_capacity(&mut power, Some(42));
    assert_eq!(power.batteries.unwrap()[0].charge_percent, None);
}

#[test]
fn binder_reply_rejects_errors_missing_fields_and_invalid_capacity() {
    let decode = |words: &[i32], value: Option<i64>| {
        let mut words = words.iter().copied();
        let mut power = PowerInfo::default();
        let reply = decode_battery_reply(|| words.next(), || value);
        merge_capacity(&mut power, reply);
        power
            .batteries
            .and_then(|b| b.into_iter().next())
            .and_then(|b| b.charge_percent)
    };
    assert_eq!(decode(&[0, 0, 1], Some(42)), Some(42));
    for words in [
        &[][..],
        &[0][..],
        &[0, 0][..],
        &[1, 0, 1][..],
        &[0, -1, 1][..],
        &[0, 0, 0][..],
        &[0, 0, 2][..],
    ] {
        assert_eq!(decode(words, Some(42)), None, "{words:?}");
    }
    for value in [None, Some(-1), Some(101), Some(i64::MIN), Some(i64::MAX)] {
        assert_eq!(decode(&[0, 0, 1], value), None);
    }
}
