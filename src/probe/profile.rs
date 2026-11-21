use crate::probe::{BatteryKind, PowerInfo};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::IsTerminal;

/// Chassis class. Says nothing about displays or the current power source.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFactor {
    /// Portable machine with an internal battery.
    Laptop,
    /// Stationary machine, including all-in-ones and minis.
    Desktop,
    /// Rack, blade, or other server chassis.
    Server,
    /// Tablet or detachable.
    Tablet,
    /// Handheld phone, foldables included.
    Phone,
    /// A recognized chassis that fits none of the above.
    Other,
    /// No evidence, or evidence that names no chassis. Not a desktop.
    #[default]
    Unknown,
}

/// What [`FormFactor`] was decided from, strongest first.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormFactorEvidence {
    /// SMBIOS/DMI chassis type.
    Firmware,
    /// A known model prefix such as `MacBook` or `Macmini`.
    ModelIdentifier,
    /// A heuristic, never used to infer desktop from an absent battery.
    SystemBattery,
    /// An OS-declared device class, such as Android's
    /// `ro.build.characteristics`.
    SystemProperty,
    /// Nothing to go on; the form factor is `Unknown`.
    #[default]
    Unknown,
}

/// OS-reported displays, independent of graphical-session environment variables.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayState {
    /// At least one display, physical or virtual, is attached.
    Connected,
    /// Headless according to the available display interface. Virtual displays
    /// count as connected; an inaccessible interface is `Unknown`, not headless.
    NoneDetected,
    /// The display interface could not be queried.
    #[default]
    Unknown,
}

/// Best-effort environment detection, not a security boundary or attestation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironment {
    /// A container or jail. Host chassis, display, power, and thermal facts
    /// are left unknown rather than read through to the host.
    Container,
    /// A hypervisor guest. Reported facts describe the virtual machine.
    VirtualMachine,
    /// No virtualization/container marker found; not proof of bare metal.
    NoneDetected,
    /// The platform's guest detection gave no answer.
    #[default]
    Unknown,
}

/// Session hints do not establish whether the machine itself is headless.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SessionContext {
    /// `SSH_CONNECTION`, `SSH_CLIENT`, or `SSH_TTY` is set in this process.
    pub ssh: bool,
    /// `WAYLAND_DISPLAY` or `DISPLAY` is set in this process.
    pub graphical_hint: bool,
    /// Standard input is a terminal.
    pub terminal_attached: bool,
}

impl SessionContext {
    pub(crate) fn local() -> Self {
        Self {
            ssh: ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
                .into_iter()
                .any(env_non_empty),
            graphical_hint: ["WAYLAND_DISPLAY", "DISPLAY"]
                .into_iter()
                .any(env_non_empty),
            terminal_attached: std::io::stdin().is_terminal(),
        }
    }
}

/// Set to valid Unicode other than whitespace; a non-Unicode value is ignored.
fn env_non_empty(key: &str) -> bool {
    std::env::var(key).is_ok_and(|value| !value.trim().is_empty())
}

/// What kind of machine this is and how this process is attached to it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MachineProfile {
    /// Chassis class.
    pub form_factor: FormFactor,
    /// What `form_factor` was decided from.
    pub form_factor_evidence: FormFactorEvidence,
    /// Vendor model: macOS `hw.model`, Linux DMI product name, FreeBSD SMBIOS
    /// product name from kenv. `None` when unreadable or in a container.
    pub model: Option<String>,
    /// Whether displays are attached.
    pub displays: DisplayState,
    /// Container, virtual machine, or neither detected.
    pub execution_environment: ExecutionEnvironment,
    /// Session hints from this process's environment.
    pub session: SessionContext,
    /// Raw FreeBSD driver observations, not evidence of display attachment.
    /// Absent on other platforms and when host observations are suppressed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub freebsd_acpi: Option<FreebsdAcpiDiagnostics>,
}

/// Read-only ACPI driver values, independent of chassis or display classification.
///
/// Keys are OS-local sysctl names, not stable hardware identifiers. `None` means
/// enumeration failed or the interface is unavailable; a `None` value means that
/// leaf could not be read. Unrecognized values are preserved, not normalized.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FreebsdAcpiDiagnostics {
    /// `dev.acpi_lid.<unit>.state`: 0 closed, 1 open according to the driver,
    /// which caches it and assumes open when the firmware query fails. Not
    /// physical lid position or sensor health.
    pub driver_lid_states: Option<BTreeMap<String, Option<i32>>>,
    /// `hw.acpi.video.<output>.active`: 0 inactive, 1 active according to the
    /// driver. Missing firmware `_DCS` reads active; other failures may read
    /// inactive. Neither proves attachment or headlessness.
    pub driver_video_activity: Option<BTreeMap<String, Option<i32>>>,
}

pub fn classify(
    chassis: Option<u8>,
    model: Option<&str>,
    power: &PowerInfo,
) -> (FormFactor, FormFactorEvidence) {
    // SMBIOS chassis types. Unknown/unspecified values are not desktops.
    let firmware = match chassis {
        Some(3..=7 | 13 | 15 | 16 | 35 | 36) => Some(FormFactor::Desktop),
        Some(8..=11 | 14 | 31 | 32) => Some(FormFactor::Laptop),
        Some(17 | 23 | 28 | 29) => Some(FormFactor::Server),
        Some(30) => Some(FormFactor::Tablet),
        Some(1 | 12 | 18..=22 | 24..=27 | 33 | 34) => Some(FormFactor::Other),
        _ => None,
    };
    if let Some(form_factor) = firmware {
        return (form_factor, FormFactorEvidence::Firmware);
    }
    if let Some(model) = model {
        if model.starts_with("MacBook") || model.starts_with("PowerBook") {
            return (FormFactor::Laptop, FormFactorEvidence::ModelIdentifier);
        }
        if ["Macmini", "MacPro", "MacStudio", "iMac", "PowerMac"]
            .iter()
            .any(|prefix| model.starts_with(prefix))
        {
            return (FormFactor::Desktop, FormFactorEvidence::ModelIdentifier);
        }
    }
    if power.batteries.as_ref().is_some_and(|batteries| {
        batteries
            .iter()
            .any(|battery| battery.kind == BatteryKind::System && battery.present == Some(true))
    }) {
        return (FormFactor::Laptop, FormFactorEvidence::SystemBattery);
    }
    (FormFactor::Unknown, FormFactorEvidence::Unknown)
}

#[cfg(test)]
mod tests;
