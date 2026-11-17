# vitals-machines

> **AMS Santorio**, NCC-4238, *Santorio*-class vitals tender.

Sick-bay flight manual: take the host's vitals, and do not call a warning over
until the vitals have held.

Santorio Santorio put a scale on the thermometer and a pendulum to the pulse,
then weighed himself for thirty years before drawing conclusions. The tender
keeps the same discipline for a machine: measure what is there, call a missing
reading unknown, and wait for stability before declaring recovery. The fleet
needed it because the `ChaOS` kernel carried all of this inline. A warning that
clears the moment a reading dips under its threshold flaps, and a host that
slept through the dip must not wake up recovered.

Allocation-free `no_std`, no `unsafe` in the core, no clock, no I/O. The host
probes sit behind the `probe` feature. This is a Rust-only crate with no Ruby
gem.

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
