#![doc = include_str!("../README.md")]
#![cfg_attr(not(any(test, feature = "probe")), no_std)]
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

use core::time::Duration;

use feedback_machines::{Dwell, Hold};
use periscope_machines::Issuer;

#[cfg(feature = "probe")]
#[allow(unsafe_code)]
pub mod probe;

/// Where a recovery episode stands. Absence of a warning is not recovery.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Phase {
    /// No episode has opened: no warning was ever sampled.
    #[default]
    Healthy,
    /// An episode is open and the vitals have not cleared their recovery margin.
    Warning,
    /// Every outstanding condition has headroom, but not for long enough yet.
    Stabilizing,
    /// Headroom held for the required span. A later lapse (`Short`,
    /// `Unavailable`, `Blocked`, a gap) leaves this phase within the same
    /// episode; only a [`Sample::Warning`] after recovery opens the next one.
    Recovered,
    /// An episode is open and the vitals could not be measured.
    Unavailable,
    /// An episode is open and its recovery target cannot be reached as configured.
    Blocked,
}

/// What the caller's policy concluded from one fresh observation.
///
/// The machine never sees readings, only this verdict. With no episode open,
/// every sample except [`Sample::Warning`] leaves the machine [`Phase::Healthy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sample {
    /// At least one warning threshold fires now.
    Warning,
    /// Nothing fires, but an outstanding condition is still inside its
    /// recovery margin (below threshold, not yet back to safety).
    Short,
    /// Every outstanding condition is past its recovery margin. `required` is
    /// how long that must hold; it may differ per episode (thermal recovery
    /// wants minutes, a charger plugged in can count at once).
    Headroom {
        /// Continuous headroom needed before the episode counts as recovered.
        required: Duration,
    },
    /// The vitals an outstanding condition depends on could not be measured.
    Unavailable,
    /// An outstanding condition's recovery target is unreachable (a margin
    /// above 100% charge, more free space than the filesystem holds).
    Blocked,
}

/// Why an observation was not honoured. Nothing changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// `now` is earlier than the latest honoured observation.
    TimeRegression,
    /// No episode id is left to open a new episode with.
    Exhausted,
}

/// One warning episode: from the warning that opened it until the next
/// warning after it recovered.
///
/// Issued through a [`periscope_machines::Issuer`],
/// so it carries the recovery's scope and is never reissued. Use it to tie
/// per-episode work (a checkpoint, a notification) to the episode it was for:
/// a slow write must not land on a newer one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Episode(u64);

impl Episode {
    /// The id as a plain integer: scope in the top 16 bits, never zero.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Hysteresis for host warnings: an episode opens on the first warning and is
/// only [`Phase::Recovered`] after continuous headroom for the required span.
///
/// The window is a [`feedback_machines::Dwell`]: a silence longer than
/// `max_gap` between observations restarts it, because the host may have slept
/// and nobody was watching. Feed a clock that keeps counting while the host is
/// suspended (`CLOCK_BOOTTIME`, `mach_continuous_time`); with one that stops,
/// a sleep looks like no gap at all and the window survives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recovery {
    phase: Phase,
    episodes: Issuer,
    window: Dwell,
    last: Option<Duration>,
    recovered_episode: bool,
}

impl Recovery {
    /// A machine in scope zero with no episode open and nothing observed.
    /// `max_gap` is the longest silence that still counts as continuous.
    #[must_use]
    pub const fn new(max_gap: Duration) -> Self {
        Self::with_scope(0, max_gap)
    }

    /// As [`Recovery::new`], issuing episodes in `scope`. Give each recovery
    /// whose episodes meet in one place its own scope.
    #[must_use]
    pub const fn with_scope(scope: u16, max_gap: Duration) -> Self {
        Self {
            phase: Phase::Healthy,
            episodes: Issuer::new(scope),
            window: Dwell::new(max_gap),
            last: None,
            recovered_episode: false,
        }
    }

    /// Fold one sample taken at `now` into the episode.
    ///
    /// Observations inside an episode, and the warning that opens one, record
    /// `now`. With no episode open, any other sample answers
    /// [`Phase::Healthy`] whatever the time and records nothing.
    ///
    /// # Errors
    ///
    /// [`Refusal::TimeRegression`] if `now` precedes the latest recorded
    /// observation, [`Refusal::Exhausted`] if a warning would open an episode
    /// with no id left. Nothing changed.
    pub fn observe(&mut self, now: Duration, sample: Sample) -> Result<Observed, Refusal> {
        let elapsed = match self.last {
            Some(last) => now.checked_sub(last).ok_or(Refusal::TimeRegression)?,
            None => Duration::ZERO,
        };
        let open = self.phase != Phase::Healthy;
        let opens = matches!(sample, Sample::Warning) && (!open || self.recovered_episode);
        if !open && !opens {
            return Ok(Observed {
                phase: Phase::Healthy,
                episode: self.episode(),
                opened: false,
            });
        }
        let mut episodes = self.episodes;
        if opens {
            episodes.issue().map_err(|refusal| match refusal {
                periscope_machines::Refusal::Exhausted => Refusal::Exhausted,
            })?;
        }
        let mut window = self.window.clone();
        let phase = Self::phase_after(&mut window, elapsed, sample)?;

        // Decided: from here on the observation is honoured.
        if opens {
            self.recovered_episode = false;
        }
        self.episodes = episodes;
        self.window = window;
        self.phase = phase;
        self.last = Some(now);
        self.recovered_episode |= phase == Phase::Recovered;
        Ok(Observed {
            phase,
            episode: self.episode(),
            opened: opens,
        })
    }

    fn phase_after(
        window: &mut Dwell,
        elapsed: Duration,
        sample: Sample,
    ) -> Result<Phase, Refusal> {
        let (holds, required) = match sample {
            Sample::Headroom { required } => (true, required),
            _ => (false, Duration::ZERO),
        };
        // The window only adds spans between recorded instants, so it never
        // holds past `now`, let alone `Duration::MAX`: feedback cannot refuse.
        let hold = window
            .observe(holds, elapsed, required)
            .map_err(|refusal| match refusal {
                feedback_machines::Refusal::NotFinite
                | feedback_machines::Refusal::InvalidDuration
                | feedback_machines::Refusal::Overflow => {
                    debug_assert!(false, "a dwell over recorded instants overflowed");
                    Refusal::TimeRegression
                }
            })?;
        Ok(match (sample, hold) {
            (Sample::Unavailable, _) => Phase::Unavailable,
            (Sample::Blocked, _) => Phase::Blocked,
            (_, Hold::Satisfied) => Phase::Recovered,
            (_, Hold::Holding) => Phase::Stabilizing,
            (_, Hold::Interrupted) => Phase::Warning,
        })
    }

    /// Forget the open episode and the observed time, as if new, but keep
    /// issuing from where the issuer left off.
    ///
    /// For when what is being watched changes (another workspace, another
    /// config): the old episode is closed for good, and a later warning opens
    /// one with a fresh id, so work still carrying a pre-reset [`Episode`]
    /// never matches it, even if the old context comes back. Any `now` is
    /// accepted after a reset.
    pub const fn reset(&mut self) {
        self.phase = Phase::Healthy;
        self.window = Dwell::new(self.window.max_gap());
        self.last = None;
        self.recovered_episode = false;
    }

    /// The current phase.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// The current episode, `None` before the first warning and after a
    /// [`Recovery::reset`] until the next one.
    #[must_use]
    pub const fn episode(&self) -> Option<Episode> {
        // Healthy only before the first warning or after a reset; either way
        // no episode is current, whatever the issuer handed out last.
        if matches!(self.phase, Phase::Healthy) {
            return None;
        }
        match self.episodes.last() {
            Some(raw) => Some(Episode(raw)),
            None => None,
        }
    }

    /// Whether an episode is open and has not recovered.
    #[must_use]
    pub const fn unresolved(&self) -> bool {
        !matches!(self.phase, Phase::Healthy | Phase::Recovered)
    }

    /// How long headroom has held as of the latest observation, `None` when
    /// no window is running.
    #[must_use]
    pub const fn stable_for(&self) -> Option<Duration> {
        self.window.held()
    }

    /// The latest honoured observation instant.
    #[must_use]
    pub const fn last_observed(&self) -> Option<Duration> {
        self.last
    }

    /// The longest silence that still counts as continuous.
    #[must_use]
    pub const fn max_gap(&self) -> Duration {
        self.window.max_gap()
    }
}

/// The answer to an honoured observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Observed {
    /// The phase after this observation.
    pub phase: Phase,
    /// The current episode; `None` before the first warning and after a
    /// [`Recovery::reset`] until the next one.
    pub episode: Option<Episode>,
    /// Whether this observation opened [`Observed::episode`]. The caller
    /// resets whatever it tracks per episode (outstanding conditions).
    pub opened: bool,
}

#[cfg(test)]
mod tests;
