# vitals-machines

> **AMS Santorio**, NCC-4238, *Santorio*-class vitals tender.

Sick-bay flight manual: take the host's vitals, and do not call it recovered
until they have held.

Santorio Santorio put a scale on the thermometer and a pendulum to the pulse,
then weighed himself for thirty years before drawing conclusions. The tender
does the same for a machine: measure what is there, call missing readings
unknown, and wait for stability before declaring a warning over.

The core is an allocation-free `no_std` recovery machine with no `unsafe`. It
reads no clock and does no I/O. The `probe` feature adds the host probes. This
is a Rust-only crate with no Ruby gem.

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
  (every outstanding condition is past its margin), `Unavailable` or `Blocked`.
  Per-condition thresholds with a recovery margin are
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
  one (`Observed::opened`). A lapse without a warning (`Short`, `Unavailable`,
  `Blocked`, a gap) leaves `Recovered` but stays in the same episode. Episode ids come from a
  `periscope_machines::Issuer` (`Recovery::with_scope`); tie per-episode work
  such as a checkpoint to its `Episode` so a slow write cannot land on a newer
  one.
- **Time.** `now` is a `Duration` since an epoch the caller picks.
  Observations inside an episode, and the warning that opens one, record it;
  with no episode open, other samples answer `Healthy` and record nothing.
  `required` and `max_gap` are spans.
- **Refusals change nothing.** `TimeRegression` (`now` before the latest
  recorded observation) and `Exhausted` (no episode id left). The machine compares equal to a clone
  taken before the call.
- **Exhaustive.** `Phase`, `Sample` and `Refusal` are exhaustive enums; a new
  variant is a breaking change.

## Probes (`probe` feature)

Fresh, best-effort, synchronous observations on Linux, macOS and FreeBSD. No
thresholds, notifications, polling or configuration: probes measure, the
caller decides. Use a blocking worker in async code. Timestamps mark
collection start, not an atomic observation.

`inspect_machine()` answers form factor, displays, session hints, execution
environment, power and thermals. `inspect_storage(&[StorageTarget])` probes
only the paths you pass. `inspect_indexing(workspace)` reports Spotlight and
known indexer processes.

- **Independent facts.** Chassis, displays, session and power source are
  separate. A headless laptop on external power is valid. A missing battery
  does not imply a desktop. Known containers and jails do not report the
  host's chassis, displays, batteries or thermals as their own.
- **Unknown is not false.** Missing, faulted, unreadable or implausible values
  are unknown, never cool, charged or empty. `PowerInfo::on_battery_power()`
  is false for a dead battery on external power. Multiple batteries stay
  separate; no averaged system percentage is invented.
- **Thermals.** The strongest recognized OS thermal state plus CPU-associated
  channels, each with its OS identifier, label, source, kind, value and the
  driver's critical limit. AMD Tctl is `control`, not die temperature.
  Channels are not averaged or deduplicated. macOS reports OS thermal pressure
  (IOPM and `NSProcessInfo`, raw values in `thermal.macos`), not Celsius; IOPM
  levels are not ordered numerically (0 normal, 100 danger, 10 crisis).
- **Task-scoped storage.** Each path resolves through symlinks to its
  filesystem; a future output probes its nearest existing ancestor. Paths
  sharing filesystem identity and read-only view are grouped. No free space is
  summed across disks. Probe failures stay separate from zero free space.
  Paths must be absolute; dangling symlinks and ambiguous `..` paths are
  errors. `backing: other` is not a durability guarantee.
- **Commands.** Absolute executable paths, the C locale, no shell, no sudo,
  bounded output, a one-second deadline each; timed-out children are killed
  and reaped. No settings, services or indexes are changed.

| | Linux | macOS | FreeBSD |
|---|---|---|---|
| Form factor | SMBIOS chassis; system-battery fallback | Model identifier; system-battery fallback | System-battery fallback |
| Power | sysfs power supplies | `pmset -g batt`, two-second bound | ACPI sysctls |
| Displays | DRM connector status | CoreGraphics online display list | Unknown |
| CPU temperature | hwmon (`coretemp`, `k10temp`, `k8temp`, `via_cputemp`) and CPU thermal zones | Unavailable | CPU temperature/Tjmax sysctls |
| OS thermal state | Unknown | IOPM warnings, `NSProcessInfo` pressure | Unknown |
| Storage | `statvfs` / `statfs` | `statvfs` / `statfs` | `statvfs` / `statfs` |
| Indexing | Tracker/LocalSearch, Baloo, `updatedb` processes | `mdutil -s` on the workspace volume, Spotlight processes | Unsupported |

Detection is best-effort, not proof that every sensor was enumerated or that
cooling is safe. No private macOS interfaces, privileged subprocesses or
driver loading are used.

Extracted from the `chaos-machine`
crate and the recovery policy of the `ChaOS` kernel.
