use crate::probe::ExecutionEnvironment;
use serde::Serialize;

/// Strongest recognized system-wide OS thermal warning/pressure state.
/// Never inferred from individual temperatures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ThermalState {
    /// Unavailable, unsupported, or unrecognized; not evidence of safe cooling.
    #[default]
    Unknown,
    /// The OS reports no thermal warning, not a guarantee about every component.
    Normal,
    /// The OS reports elevated thermal pressure (macOS danger/serious).
    Warning,
    /// The OS reports critical thermal pressure (macOS crisis/critical).
    Critical,
}

/// OS interface a temperature channel was read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TemperatureSource {
    /// Linux `/sys/class/hwmon`.
    Hwmon,
    /// Linux `/sys/class/thermal` thermal zone.
    ThermalZone,
    /// FreeBSD sysctl.
    Sysctl,
}

/// What a channel's degrees measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TemperatureKind {
    /// A physical temperature in Celsius.
    Physical,
    /// A driver-defined control scale, such as AMD Tctl, not die temperature.
    Control,
    /// The driver does not say which; do not compare against physical limits.
    Unknown,
}

/// One CPU-associated channel, not an average or a unique physical sensor.
#[derive(Debug, Clone, Serialize)]
pub struct CpuTemperature {
    /// OS-local sysfs path or sysctl name; not stable across boots/namespaces.
    pub id: String,
    /// Human-readable channel name (`k10temp: Tctl`, a thermal zone type,
    /// `CPU 0`); not a stable key.
    pub label: String,
    /// Interface the channel was read from.
    pub source: TemperatureSource,
    /// Whether `celsius` is a physical temperature or a control scale.
    pub kind: TemperatureKind,
    /// Celsius, or degree units on a control scale when `kind` is `Control`.
    /// Missing, unreadable, disabled, faulted, or implausible values stay `None`.
    pub celsius: Option<f64>,
    /// Driver/OS-reported limit on this channel's scale, not a configured policy.
    pub critical_celsius: Option<f64>,
}

/// Thermal observations from one sample. Default is everything unknown.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ThermalInfo {
    /// System-wide OS thermal state; `Unknown` in containers and where unsupported.
    pub state: ThermalState,
    /// Best-effort CPU channels only. Empty means none could be identified,
    /// not a cool CPU or a complete hardware inventory. Channels may overlap.
    pub cpu_temperatures: Vec<CpuTemperature>,
    /// Raw macOS observations from this sample, not additional safety evidence.
    /// Absent on other platforms and when host observations are suppressed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub macos: Option<MacosThermalDiagnostics>,
    /// Raw Android thermal-service observations from this sample. Absent on
    /// other platforms, and where the service could not be reached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub android: Option<AndroidThermalDiagnostics>,
}

/// Preserve the two macOS sources separately so conflicting or process-local
/// observations remain visible rather than only their merged severity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MacosThermalDiagnostics {
    /// `IOReturn` from `IOPMGetThermalWarningLevel`; zero means the query succeeded.
    pub iopm_return_code: i32,
    /// Raw successful IOPM level: 0 normal, 100 danger, 10 crisis. Other values
    /// are unrecognized. None means the query failed, not a normal reading.
    pub iopm_warning_level: Option<u32>,
    /// Raw `NSProcessInfo` value: 0 nominal (also returned when unsupported),
    /// 1 fair, 2 serious, 3 critical. Unknown values are preserved verbatim.
    pub ns_process_info_thermal_state: isize,
}

/// What Android's thermal service (`AThermalManager`, NDK) reported.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct AndroidThermalDiagnostics {
    /// `AThermal_getCurrentThermalStatus`: -1 error, 0 none, 1 light,
    /// 2 moderate, 3 severe, 4 critical, 5 emergency, 6 shutdown.
    pub status: i32,
    /// `AThermal_getThermalHeadroom` with no forecast: 0.0 is no throttling,
    /// 1.0 is severe throttling. Android 12 and later; `None` where the
    /// platform has no headroom or would not say.
    pub headroom: Option<f32>,
}

pub fn inspect(environment: ExecutionEnvironment) -> ThermalInfo {
    if environment == ExecutionEnvironment::Container {
        return ThermalInfo::default();
    }
    crate::probe::platform::read_thermal()
}

// Generous CPU-sensor sanity bounds, not warning thresholds. Preserve valid
// zero/subzero readings and overheating; reject sentinel/garbage values.
#[cfg(any(
    target_os = "linux",
    target_os = "android",
    target_os = "freebsd",
    test
))]
pub fn valid_celsius(value: f64) -> Option<f64> {
    (-100.0..=250.0).contains(&value).then_some(value)
}

#[cfg(test)]
mod tests;
