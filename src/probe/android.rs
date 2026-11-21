//! Android: a Linux kernel behind an app sandbox.
//!
//! The kernel's sysfs is Linux's, so its readers are reused, but what a
//! process may read is the vendor's SELinux policy. Measured from an app's
//! domain on a Galaxy Z Fold6 (Android 14), a Pixel 6 Pro (17) and a Galaxy
//! Tab S7 (15): `/sys/class/power_supply` is denied on all three, the
//! thermal zones are readable on the Galaxies and denied on the Pixel. The
//! shell reads more (power on the Pixel and the Tab). Every reader is best
//! effort, and a denial is an unknown, never a healthy reading. The thermal
//! state comes from the NDK's thermal service, the one source an app can
//! rely on (Android 11+). An app's battery is the platform's to report
//! (`BatteryManager`), unless the experimental `android-binder` fallback is enabled.

use crate::probe::linux::thermal::zones;
use crate::probe::linux::{adapter, battery, boolean, text};
use crate::probe::power::source_from_supplies;
use crate::probe::{BatteryKind, FormFactor, FormFactorEvidence, PowerInfo, ThermalState};
use std::fs;
use std::path::Path;

#[cfg(all(target_os = "android", feature = "android-binder"))]
mod binder;

#[cfg(target_os = "android")]
use crate::probe::{DisplayState, ExecutionEnvironment, MachineProfile, ThermalInfo};

#[cfg(target_os = "android")]
pub fn detect() -> (MachineProfile, PowerInfo) {
    let power = read_power(Path::new("/sys/class/power_supply"));
    #[cfg(feature = "android-binder")]
    let power = {
        let mut power = power;
        if needs_capacity(&power) {
            merge_capacity(&mut power, binder::capacity());
        }
        power
    };
    let emulator = [c"ro.kernel.qemu", c"ro.boot.qemu"]
        .iter()
        .any(|name| system::property(name).as_deref() == Some("1"));
    let (form_factor, form_factor_evidence) =
        form_factor(system::property(c"ro.build.characteristics").as_deref());
    let profile = MachineProfile {
        model: system::property(c"ro.product.model").filter(|model| !model.is_empty()),
        // A phone or a tablet has its panel built in; the DRM connectors that
        // would say so are not the app's to read.
        displays: if matches!(form_factor, FormFactor::Phone | FormFactor::Tablet) {
            DisplayState::Connected
        } else {
            DisplayState::Unknown
        },
        execution_environment: if emulator {
            ExecutionEnvironment::VirtualMachine
        } else {
            ExecutionEnvironment::NoneDetected
        },
        form_factor,
        form_factor_evidence,
        ..MachineProfile::default()
    };
    (profile, power)
}

#[cfg(any(feature = "android-binder", test))]
fn decode_battery_reply(
    mut read_i32: impl FnMut() -> Option<i32>,
    read_i64: impl FnOnce() -> Option<i64>,
) -> Option<i64> {
    if read_i32()? != 0 || read_i32()? != 0 || read_i32()? != 1 {
        return None;
    }
    read_i64()
}

#[cfg(any(feature = "android-binder", test))]
fn needs_capacity(power: &PowerInfo) -> bool {
    power.batteries.as_ref().is_none_or(|batteries| {
        batteries.iter().any(|battery| {
            battery.kind == BatteryKind::System
                && battery.present != Some(false)
                && battery.charge_percent.is_none()
        })
    })
}

#[cfg(any(feature = "android-binder", test))]
fn merge_capacity(power: &mut PowerInfo, capacity: Option<i64>) {
    let Some(capacity) = capacity
        .and_then(|value| u8::try_from(value).ok())
        .filter(|value| *value <= 100)
    else {
        return;
    };
    if let Some(batteries) = &mut power.batteries {
        if let Some(battery) = batteries
            .iter_mut()
            .find(|battery| battery.kind == BatteryKind::System && battery.present != Some(false))
            && battery.charge_percent.is_none()
        {
            battery.charge_percent = Some(capacity);
        }
    } else {
        power.batteries = Some(vec![crate::probe::BatteryInfo {
            name: "battery".into(),
            kind: BatteryKind::System,
            present: None,
            charge_percent: Some(capacity),
            state: crate::probe::BatteryState::Unknown,
        }]);
    }
}

#[cfg(target_os = "android")]
pub fn read_thermal() -> ThermalInfo {
    let service = system::thermal_service();
    ThermalInfo {
        state: service.map_or(ThermalState::Unknown, |s| thermal_state(s.status)),
        cpu_temperatures: zones(Path::new("/"), cpu_zone),
        android: service,
        ..ThermalInfo::default()
    }
}

/// Android's power supplies: the one `battery` healthd reports, and the
/// adapters (`usb`, `ac`, `wireless`, the Type-C port) by their `online`.
/// Vendors add fuel gauges, charger ICs and power-sharing nodes beside them,
/// often unreadable to an app; those are skipped rather than allowed to void
/// the battery, as the Linux reader's strictness would. An unreadable
/// supply may still be an online adapter, so it blocks "no external power"
/// but never hides one that reads online.
fn read_power(root: &Path) -> PowerInfo {
    let Ok(entries) = fs::read_dir(root) else {
        return PowerInfo::default();
    };
    let mut adapters = Vec::new();
    let mut hidden = false;
    for entry in entries {
        let Ok(entry) = entry else {
            hidden = true;
            continue;
        };
        let path = entry.path();
        match text(&path.join("type")) {
            Some(kind) if adapter(&kind) => adapters.push(boolean(&path.join("online"))),
            Some(_) => {}
            None => hidden = true,
        }
    }
    let path = root.join("battery");
    let batteries = (text(&path.join("type")).as_deref() == Some("Battery"))
        .then(|| vec![battery(&path, "battery".to_owned(), BatteryKind::System)]);
    let external_power = if adapters.contains(&Some(true)) {
        Some(true)
    } else if !hidden && !adapters.is_empty() && adapters.iter().all(|a| *a == Some(false)) {
        Some(false)
    } else {
        None
    };
    PowerInfo {
        source: source_from_supplies(external_power, batteries.as_deref()),
        external_power,
        batteries,
    }
}

/// `ro.build.characteristics` is a comma-separated device class
/// (`phone`, `tablet`, `tablet,nosdcard`, `watch`...). Plenty of phones say
/// nothing useful (a Pixel says `nosdcard`): that stays unknown.
fn form_factor(characteristics: Option<&str>) -> (FormFactor, FormFactorEvidence) {
    let traits: Vec<&str> = characteristics
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .collect();
    let found = if traits.contains(&"tablet") {
        FormFactor::Tablet
    } else if traits.contains(&"phone") {
        FormFactor::Phone
    } else if traits
        .iter()
        .any(|t| matches!(*t, "watch" | "tv" | "automotive" | "embedded"))
    {
        FormFactor::Other
    } else {
        return (FormFactor::Unknown, FormFactorEvidence::Unknown);
    };
    (found, FormFactorEvidence::SystemProperty)
}

/// CPU sensors under the names Android SoCs give their zones: Qualcomm's
/// `cpu-1-0-0`, `cpuss-0`, `cpu-0-0-usr`; Samsung Exynos and Google Tensor
/// cluster sensors `BIG`, `MID`, `LITTLE`; plain `cpu0`/`cpu-thermal`.
/// Qualcomm's `-step` zones mirror real sensors for the stepwise governor
/// and are skipped.
fn cpu_zone(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    if name.ends_with("-step") || name.ends_with("_step") {
        return false;
    }
    if matches!(name.as_str(), "big" | "mid" | "little") {
        return true;
    }
    let Some(rest) = name.strip_prefix("cpu") else {
        return false;
    };
    rest.is_empty()
        || rest.starts_with("ss")
        || rest.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '_')
}

/// The thermal service's status, as the crate's coarse state: light
/// throttling leaves the experience intact, moderate and severe throttle
/// noticeably, critical and beyond shed load or shut down.
const fn thermal_state(status: i32) -> ThermalState {
    match status {
        0 | 1 => ThermalState::Normal,
        2 | 3 => ThermalState::Warning,
        4..=6 => ThermalState::Critical,
        _ => ThermalState::Unknown,
    }
}

#[cfg(target_os = "android")]
mod system {
    use crate::probe::AndroidThermalDiagnostics;
    use std::ffi::{CStr, c_char, c_int, c_void};

    /// A system property, `None` when unset or empty.
    pub fn property(name: &CStr) -> Option<String> {
        let mut value = [0 as c_char; libc::PROP_VALUE_MAX as usize];
        // SAFETY: name is NUL-terminated; value has the PROP_VALUE_MAX bytes
        // bionic writes at most, NUL included.
        let len = unsafe { libc::__system_property_get(name.as_ptr(), value.as_mut_ptr()) };
        if len <= 0 {
            return None;
        }
        // SAFETY: bionic NUL-terminated what it wrote into value.
        let value = unsafe { CStr::from_ptr(value.as_ptr()) };
        Some(value.to_string_lossy().into_owned())
    }

    type Acquire = unsafe extern "C" fn() -> *mut c_void;
    type Release = unsafe extern "C" fn(*mut c_void);
    type Status = unsafe extern "C" fn(*mut c_void) -> c_int;
    type Headroom = unsafe extern "C" fn(*mut c_void, c_int) -> f32;

    struct Library(*mut c_void);

    impl Drop for Library {
        fn drop(&mut self) {
            // SAFETY: the handle came from a successful dlopen and is closed once.
            unsafe { libc::dlclose(self.0) };
        }
    }

    /// One sample from `AThermalManager`. Looked up at run time, so an app
    /// whose minimum is below Android 11 still loads, and reports unknown
    /// where the service is missing.
    pub fn thermal_service() -> Option<AndroidThermalDiagnostics> {
        // SAFETY: a NUL-terminated library name.
        let library = unsafe { libc::dlopen(c"libandroid.so".as_ptr(), libc::RTLD_NOW) };
        if library.is_null() {
            return None;
        }
        let library = Library(library);
        let symbol = |name: &CStr| {
            // SAFETY: a live handle and a NUL-terminated symbol name.
            let address = unsafe { libc::dlsym(library.0, name.as_ptr()) };
            (!address.is_null()).then_some(address)
        };
        // SAFETY: each symbol is the NDK function of that name, whose C
        // signature the type spells out (android/thermal.h).
        let (acquire, release, status) = unsafe {
            (
                std::mem::transmute::<*mut c_void, Acquire>(symbol(c"AThermal_acquireManager")?),
                std::mem::transmute::<*mut c_void, Release>(symbol(c"AThermal_releaseManager")?),
                std::mem::transmute::<*mut c_void, Status>(symbol(
                    c"AThermal_getCurrentThermalStatus",
                )?),
            )
        };
        // SAFETY: as above; Android 12 added it.
        let headroom = symbol(c"AThermal_getThermalHeadroom")
            .map(|address| unsafe { std::mem::transmute::<*mut c_void, Headroom>(address) });

        // SAFETY: acquire takes nothing and returns a manager or null.
        let manager = unsafe { acquire() };
        if manager.is_null() {
            return None;
        }
        // SAFETY: manager is live until the release below.
        let code = unsafe { status(manager) };
        // NaN where the device has no headroom model, or when asked more
        // than once a second.
        // SAFETY: manager is live; 0 asks for now, with no forecast.
        let headroom = headroom
            .map(|f| unsafe { f(manager, 0) })
            .filter(|h| h.is_finite());
        // SAFETY: manager came from acquire and is released once.
        unsafe { release(manager) };
        Some(AndroidThermalDiagnostics {
            status: code,
            headroom,
        })
    }
}

#[cfg(test)]
mod tests;
