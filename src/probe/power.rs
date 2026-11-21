use serde::Serialize;

/// What the machine is running on right now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerSource {
    /// Mains or an adapter, even if a battery is also discharging.
    External,
    /// A system battery is the supply. Linux and FreeBSD report this when a
    /// battery discharges with no online supply; macOS when `pmset` names
    /// battery power as the source, whatever the battery itself reports.
    Battery,
    /// A UPS battery is the supply, decided as for [`PowerSource::Battery`]
    /// (on macOS, `pmset` naming UPS power).
    Ups,
    /// Not reported or contradictory; not evidence of external power.
    #[default]
    Unknown,
}

/// Which supply a battery backs. Peripheral batteries are never reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryKind {
    /// Internal battery that powers this machine.
    System,
    /// External uninterruptible power supply the OS reports.
    Ups,
}

/// What the OS says one battery is doing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BatteryState {
    /// Taking charge.
    Charging,
    /// Supplying power.
    Discharging,
    /// Reported fully charged.
    Full,
    /// Neither charging nor discharging (charge limit, maintenance, AC hold).
    Idle,
    /// Not reported or unrecognized.
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// One system or UPS battery as the OS reports it.
pub struct BatteryInfo {
    /// OS supply name, not a hardware serial number.
    pub name: String,
    /// Whether it backs this machine or a UPS.
    pub kind: BatteryKind,
    /// Whether a battery is installed in the slot. Inferred `true` when the OS
    /// omits presence but reports live battery data (Linux) or does not say
    /// `present: false` (macOS); `None` only when nothing implies either.
    pub present: Option<bool>,
    /// Percentage 0..=100, or unknown. Never a fabricated/clamped percentage.
    pub charge_percent: Option<u8>,
    /// Charging activity, independent of the primary supply.
    pub state: BatteryState,
}

/// Current supply, not battery health. A dead/removed battery on external power
/// is not a battery-powered machine. Peripheral batteries are excluded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PowerInfo {
    /// Primary supply derived from adapter and battery reports.
    pub source: PowerSource,
    /// Adapter presence where reported. Battery activity is separate from the
    /// primary supply: a battery can discharge while the OS reports AC power.
    pub external_power: Option<bool>,
    /// `None`: enumeration unavailable/incomplete; `Some([])`: no system/UPS
    /// supplies observed. Neither says the machine must be a desktop.
    /// Multiple batteries stay separate; their percentages are not averaged.
    pub batteries: Option<Vec<BatteryInfo>>,
}

impl PowerInfo {
    /// Whether primary supply is battery/UPS-backed, independent of chassis.
    /// Unknown is not a safe/healthy power assertion.
    #[must_use]
    pub const fn on_battery_power(&self) -> Option<bool> {
        match self.source {
            PowerSource::Battery | PowerSource::Ups => Some(true),
            PowerSource::External => Some(false),
            PowerSource::Unknown => None,
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "macos", test))]
pub fn percent(text: &str) -> Option<u8> {
    text.trim().parse::<u8>().ok().filter(|value| *value <= 100)
}

#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    test
))]
pub fn source_from_supplies(
    external: Option<bool>,
    batteries: Option<&[BatteryInfo]>,
) -> PowerSource {
    // A reported online supply is not disproved by battery activity. Keep
    // maintenance/assistance discharge visible without calling the host
    // battery-powered. Notification policy belongs to the consumer.
    if external == Some(true) {
        return PowerSource::External;
    }
    let batteries = batteries.unwrap_or_default();
    for kind in [BatteryKind::System, BatteryKind::Ups] {
        if batteries.iter().any(|battery| {
            battery.kind == kind
                && battery.present != Some(false)
                && battery.state == BatteryState::Discharging
        }) {
            return match kind {
                BatteryKind::System => PowerSource::Battery,
                BatteryKind::Ups => PowerSource::Ups,
            };
        }
    }
    if external.is_none()
        && batteries.iter().any(|battery| {
            battery.present != Some(false) && battery.state == BatteryState::Charging
        })
    {
        PowerSource::External
    } else {
        PowerSource::Unknown
    }
}

#[cfg(test)]
mod tests;
