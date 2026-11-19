use super::*;

fn s(secs: u64) -> Duration {
    Duration::from_secs(secs)
}

fn headroom(secs: u64) -> Sample {
    Sample::Headroom { required: s(secs) }
}

fn phase(machine: &mut Recovery, now: u64, sample: Sample) -> Phase {
    match machine.observe(s(now), sample) {
        Ok(observed) => observed.phase,
        Err(refusal) => panic!("observation at {now}s refused: {refusal:?}"),
    }
}

#[test]
fn without_an_episode_only_a_warning_leaves_healthy() {
    let mut machine = Recovery::new(s(60));
    for sample in [
        Sample::Short,
        headroom(0),
        Sample::Unavailable,
        Sample::Blocked,
    ] {
        assert_eq!(phase(&mut machine, 10, sample), Phase::Healthy);
    }
    // Healthy answers are time-independent: nothing was recorded.
    assert_eq!(machine, Recovery::new(s(60)));
    let opened = machine.observe(s(2), Sample::Warning);
    assert_eq!(
        opened.map(|o| (o.phase, o.opened)),
        Ok((Phase::Warning, true))
    );
    assert!(machine.unresolved());
}

#[test]
fn an_episode_recovers_only_after_headroom_holds() {
    let mut machine = Recovery::new(s(60));
    phase(&mut machine, 0, Sample::Warning);
    assert_eq!(phase(&mut machine, 30, headroom(60)), Phase::Stabilizing);
    assert_eq!(
        phase(&mut machine, 40, Sample::Unavailable),
        Phase::Unavailable
    );
    assert_eq!(phase(&mut machine, 50, Sample::Blocked), Phase::Blocked);
    assert_eq!(phase(&mut machine, 60, headroom(60)), Phase::Stabilizing);
    assert_eq!(phase(&mut machine, 120, headroom(60)), Phase::Recovered);
    assert!(!machine.unresolved());
}

#[test]
fn a_warning_after_recovery_opens_the_next_episode() {
    let mut machine = Recovery::with_scope(7, s(60));
    phase(&mut machine, 0, Sample::Warning);
    let first = machine.episode();
    phase(&mut machine, 1, headroom(0));
    // Slipping back inside the margin is still the recovered episode.
    assert_eq!(phase(&mut machine, 2, Sample::Short), Phase::Warning);
    assert_eq!(machine.episode(), first);
    let next = machine.observe(s(3), Sample::Warning);
    assert_eq!(next.map(|o| o.opened), Ok(true));
    assert_ne!(machine.episode(), first);
    assert_eq!(
        machine
            .episode()
            .map(|e| periscope_machines::scope_of(e.raw())),
        Some(7)
    );
}

#[test]
fn refusals_change_nothing() {
    let mut machine = Recovery::new(s(60));
    phase(&mut machine, 10, Sample::Warning);
    phase(&mut machine, 20, headroom(60));
    let before = machine.clone();
    assert_eq!(
        machine.observe(s(19), Sample::Warning),
        Err(Refusal::TimeRegression)
    );
    assert_eq!(machine, before);

    phase(&mut machine, 30, headroom(0));
    machine.episodes = Issuer::restored(0, Some(periscope_machines::SEQUENCE_MAX))
        .unwrap_or_else(|| panic!("a spent issuer is reachable"));
    let before = machine.clone();
    assert_eq!(
        machine.observe(s(31), Sample::Warning),
        Err(Refusal::Exhausted)
    );
    assert_eq!(machine, before);
}

#[test]
fn reset_closes_the_episode_and_forgets_time() {
    let mut machine = Recovery::new(s(60));
    phase(&mut machine, 100, Sample::Warning);
    phase(&mut machine, 110, headroom(60));
    machine.reset();
    assert_eq!(machine.phase(), Phase::Healthy);
    assert_eq!(machine.episode(), None);
    assert_eq!(machine.stable_for(), None);
    assert_eq!(machine.last_observed(), None);
    assert_eq!(machine.max_gap(), s(60));
    // No episode open: anything but a warning answers Healthy.
    assert_eq!(
        machine.observe(s(120), headroom(0)),
        Ok(Observed {
            phase: Phase::Healthy,
            episode: None,
            opened: false,
        })
    );
}

#[test]
fn reset_accepts_any_now() {
    let mut machine = Recovery::new(s(60));
    phase(&mut machine, 100, Sample::Warning);
    assert_eq!(
        machine.observe(s(5), Sample::Short),
        Err(Refusal::TimeRegression)
    );
    machine.reset();
    assert_eq!(phase(&mut machine, 5, Sample::Warning), Phase::Warning);
    assert_eq!(machine.last_observed(), Some(s(5)));
}

#[test]
fn a_warning_after_reset_never_reuses_an_episode() {
    let mut machine = Recovery::with_scope(3, s(60));
    phase(&mut machine, 0, Sample::Warning);
    let before = machine.episode().map(Episode::raw);
    machine.reset();
    let opened = machine.observe(s(0), Sample::Warning);
    assert_eq!(
        opened.map(|o| (o.phase, o.opened)),
        Ok((Phase::Warning, true))
    );
    let after = machine.episode().map(Episode::raw);
    assert!(after > before, "{after:?} must come after {before:?}");
    assert_eq!(after.map(|raw| raw >> 48), Some(3));
}
