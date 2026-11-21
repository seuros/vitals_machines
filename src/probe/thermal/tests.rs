use super::*;

#[test]
fn unavailable_thermals_are_not_normal() {
    let thermal = ThermalInfo::default();
    assert_eq!(thermal.state, ThermalState::Unknown);
    assert!(thermal.cpu_temperatures.is_empty());
    assert!(thermal.macos.is_none());
    assert!(
        serde_json::to_value(&thermal)
            .unwrap()
            .get("macos")
            .is_none()
    );
}

#[test]
fn containers_do_not_report_host_thermals() {
    let thermal = inspect(ExecutionEnvironment::Container);
    assert_eq!(thermal.state, ThermalState::Unknown);
    assert!(thermal.cpu_temperatures.is_empty());
    assert!(thermal.macos.is_none());
}

#[test]
fn sanity_checks_preserve_zero_subzero_and_overheating() {
    for value in [-12.5, 0.0, 42.25, 140.0] {
        assert_eq!(valid_celsius(value), Some(value));
    }
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -273.15,
        32_767.0,
    ] {
        assert_eq!(valid_celsius(value), None);
    }
}

#[test]
fn native_machine_snapshot_includes_platform_observations() {
    let snapshot = crate::probe::inspect_machine();
    println!("{}", serde_json::to_string(&snapshot).unwrap());
    #[cfg(target_os = "freebsd")]
    let unjailed = crate::probe::sysctl::integer(c"security.jail.jailed") == Some(0);
    #[cfg(not(target_os = "freebsd"))]
    let unjailed = false;
    assert_eq!(snapshot.profile.freebsd_acpi.is_some(), unjailed);
    if cfg!(target_os = "freebsd") && !unjailed {
        assert!(snapshot.profile.model.is_none());
        assert_eq!(
            snapshot.profile.displays,
            crate::probe::DisplayState::Unknown
        );
        assert_eq!(snapshot.power, crate::probe::PowerInfo::default());
        assert!(snapshot.thermal.cpu_temperatures.is_empty());
        assert_eq!(snapshot.thermal.state, ThermalState::Unknown);
    }
    let thermal = snapshot.thermal;
    assert_eq!(thermal.macos.is_some(), cfg!(target_os = "macos"));
    for sensor in thermal.cpu_temperatures {
        assert_ne!(sensor.id, "");
        assert_ne!(sensor.label, "");
        for value in [sensor.celsius, sensor.critical_celsius]
            .into_iter()
            .flatten()
        {
            assert_eq!(valid_celsius(value), Some(value));
        }
    }
}
