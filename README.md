# vitals-machines

> **AMS Santorio**, NCC-4238, *Santorio*-class vitals tender.

Engine-room flight manual: read the hull's vitals, and hold the all-clear until
they have stayed steady.

Santorio Santorio put a scale on the thermometer and a pendulum to the pulse,
then weighed himself for thirty years before drawing conclusions. The crate
keeps the same discipline for a machine: measure what is there, call a missing
reading unknown, and wait for stability before declaring recovery.

Anything that runs heavy work on someone's machine (a build farm agent, a
backup job, an indexer, a local model) should back off when the host runs hot,
drains its battery or fills its disk. Pausing is easy. Knowing when to resume
is not: resume the moment a reading crosses back over its threshold and the
work flaps on and off; trust a laptop that slept through the dip and it wakes up
"recovered" with nobody having watched it cool. This crate takes the readings,
and keeps the books on when a warning is really over. What counts as a warning
stays yours.

Allocation-free `no_std`, no `unsafe` in the core, no clock, no I/O. The host
probes sit behind the `probe` feature. Rust only.

```rust
use core::time::Duration;
use vitals_machines::{Phase, Recovery, Refusal, Sample};

let s = Duration::from_secs;
let mut recovery = Recovery::new(s(60));
let cooled = Sample::Headroom { required: s(60) };

// No episode: only a warning moves the machine.
assert_eq!(recovery.observe(s(0), Sample::Short).map(|o| o.phase), Ok(Phase::Healthy));

let opened = recovery.observe(s(10), Sample::Warning);
assert_eq!(opened.map(|o| o.opened), Ok(true));

// Headroom must hold, continuously, for the required span.
let phase = |r: &mut Recovery, at| r.observe(s(at), cooled).map(|o| o.phase);
assert_eq!(phase(&mut recovery, 40), Ok(Phase::Stabilizing));
assert_eq!(phase(&mut recovery, 200), Ok(Phase::Stabilizing)); // a 160s gap restarts it
assert_eq!(phase(&mut recovery, 230), Ok(Phase::Stabilizing));
assert_eq!(phase(&mut recovery, 260), Ok(Phase::Recovered));

assert_eq!(recovery.observe(s(1), Sample::Warning), Err(Refusal::TimeRegression));
```

## Putting it together

Probe, turn the readings into a `Sample` with your own thresholds, and feed the
recovery machine. Here heavy work pauses under thermal pressure or below 10%
battery, and resumes once the host has been cool and above 20% for five
minutes.

```rust,no_run
# #[cfg(feature = "probe")]
# fn main() {
use std::time::{Duration, Instant};
use vitals_machines::probe::{BatteryKind, MachineSnapshot, ThermalState, inspect_machine};
use vitals_machines::{Phase, Recovery, Sample};

fn sample(vitals: &MachineSnapshot) -> Sample {
    if matches!(vitals.thermal.state, ThermalState::Warning | ThermalState::Critical) {
        return Sample::Warning;
    }
    let charge = vitals.power.batteries.iter().flatten()
        .filter(|battery| battery.kind == BatteryKind::System)
        .filter_map(|battery| battery.charge_percent)
        .min();
    match (vitals.power.on_battery_power(), charge) {
        (Some(false), _) => Sample::Headroom { required: Duration::from_secs(300) },
        (Some(true), Some(charge)) if charge < 10 => Sample::Warning,
        (Some(true), Some(charge)) if charge < 20 => Sample::Short,
        (Some(true), Some(_)) => Sample::Headroom { required: Duration::from_secs(300) },
        _ => Sample::Unavailable,
    }
}

let start = Instant::now();
let mut recovery = Recovery::new(Duration::from_secs(60));
loop {
    let vitals = inspect_machine();
    let now = start.elapsed();
    if let Ok(observed) = recovery.observe(now, sample(&vitals)) {
        let allowed = matches!(observed.phase, Phase::Healthy | Phase::Recovered);
        println!("heavy work allowed: {allowed}"); // pause or resume your workers
    }
    std::thread::sleep(Duration::from_secs(30));
}
# }
# #[cfg(not(feature = "probe"))]
# fn main() {}
```

`Instant` stops while the host sleeps on some platforms. If a suspend must
count as a gap, read a clock that keeps counting (`CLOCK_BOOTTIME` on Linux,
`mach_continuous_time` on macOS).

## Contract

- **Samples, not readings.** The machine never sees a temperature or a battery
  percentage. The caller's policy turns fresh readings into a `Sample`:
  `Warning` (a threshold fires), `Short` (nothing fires, but an outstanding
  condition is still inside its recovery margin), `Headroom { required }`
  (every outstanding condition is past its margin), `Unavailable` or
  `Blocked`. Per-condition thresholds with a recovery margin are
  `feedback_machines::Hysteresis`.
- **Absence of a warning is not recovery.** An episode opens on the first
  `Warning` and is `Recovered` only after `Headroom` held continuously for
  `required`. `Short`, `Unavailable`, `Blocked` and `Warning` restart the
  window. With no episode open, every sample but `Warning` stays `Healthy`.
- **The window is a `feedback_machines::Dwell`.** A silence longer than
  `max_gap` between observations restarts it: the host may have slept. Feed a
  clock that keeps counting through suspend (`CLOCK_BOOTTIME`,
  `mach_continuous_time`), or a sleep looks like no gap at all.
- **Episodes.** Once an episode has recovered, the next `Warning` opens a new
  one (`Observed::opened`). A lapse without a warning (`Short`,
  `Unavailable`, `Blocked`, a gap) leaves `Recovered` but stays in the same
  episode. Episode ids come from a `periscope_machines::Issuer`
  (`Recovery::with_scope`); tie per-episode work such as a checkpoint to its
  `Episode` so a slow write cannot land on a newer one.
- **Time.** `now` is a `Duration` since an epoch the caller picks.
  Observations inside an episode, and the warning that opens one, record it;
  with no episode open, other samples answer `Healthy` and record nothing.
  `required` and `max_gap` are spans.
- **Refusals change nothing.** `TimeRegression` (`now` before the latest
  recorded observation) and `Exhausted` (no episode id left). The machine
  compares equal to a clone taken before the call.
- **Exhaustive.** `Phase`, `Sample` and `Refusal` are exhaustive enums; a new
  variant is a breaking change.

## Probes

The `probe` feature adds fresh, best-effort, synchronous observations on
Linux, macOS and FreeBSD. `inspect_machine()` answers form factor, displays,
session hints, execution environment, power and thermals.
`inspect_storage(&[StorageTarget])` probes only the paths it is given.
`inspect_indexing(workspace)` reports Spotlight and known indexer processes.
Run them on a blocking worker in async code; a timestamp marks the start of
collection, not an atomic observation.

- **Independent facts.** Chassis, displays, session and power source are
  separate. A headless laptop on external power is valid, and a missing
  battery does not imply a desktop. Known containers and jails do not report
  the host's chassis, displays, batteries or thermals as their own.
- **Unknown is not false.** Missing, faulted, unreadable or implausible values
  are unknown, never cool, charged or empty. `PowerInfo::on_battery_power()`
  is false for a dead battery on external power. Batteries stay separate; no
  averaged system percentage is invented.
- **Thermals.** The strongest recognized OS thermal state plus CPU-associated
  channels, each with its OS identifier, label, source, kind, value and the
  driver's critical limit. AMD Tctl is `control`, not die temperature.
  Channels are not averaged or deduplicated. macOS reports thermal pressure
  (IOPM and `NSProcessInfo`, raw values in `thermal.macos`), not Celsius;
  IOPM levels are not ordered numerically (0 normal, 100 danger, 10 crisis).
- **Task-scoped storage.** Each path resolves through symlinks to its
  filesystem, and a future output probes its nearest existing ancestor. Paths
  sharing a filesystem and read-only view are grouped, free space is never
  summed across disks, and a probe failure is not zero free space. Paths must
  be absolute. `backing: other` is not a durability guarantee.
- **Commands.** Absolute executable paths, the C locale, no shell, no sudo,
  bounded output and a one-second deadline each; a timed-out child is killed
  and reaped.

| | Linux | macOS | FreeBSD |
|---|---|---|---|
| Form factor | SMBIOS chassis; system-battery fallback | Model identifier; system-battery fallback | System-battery fallback |
| Power | sysfs power supplies | `pmset -g batt`, two-second bound | ACPI sysctls |
| Displays | DRM connector status | CoreGraphics online display list | Unknown |
| CPU temperature | hwmon (`coretemp`, `k10temp`, `k8temp`, `via_cputemp`) and CPU thermal zones | Unavailable | CPU temperature/Tjmax sysctls |
| OS thermal state | Unknown | IOPM warnings, `NSProcessInfo` pressure | Unknown |
| Storage | `statvfs` / `statfs` | `statvfs` / `statfs` | `statvfs` / `statfs` |
| Indexing | Tracker/LocalSearch, Baloo, `updatedb` processes | `mdutil -s` on the workspace volume, Spotlight processes | Unsupported |

## Not Included

No thresholds, notifications, polling, configuration or remediation: probes
measure, the caller decides, and the recovery machine only says where an
episode stands. No Windows. No private macOS interfaces, privileged
subprocesses or driver loading. Detection is best-effort, not proof that every
sensor was enumerated or that cooling is safe.

## License

MIT
