use super::*;

const HEALTHY: &str = "zroot\t126701535232\t83919802368\t42781732864\t66\tONLINE\toff\n";

fn found(observation: PoolObservation) -> ZfsPool {
    match observation {
        PoolObservation::Found(pool) => pool,
        other => panic!("expected a pool, got {other:?}"),
    }
}

#[test]
fn healthy_pool_parses_into_plain_data() {
    let pool = found(parse_pool("zroot", HEALTHY));
    assert_eq!(pool.name, "zroot");
    assert_eq!(pool.size_bytes, 126_701_535_232);
    assert_eq!(pool.allocated_bytes, 83_919_802_368);
    assert_eq!(pool.free_bytes, 42_781_732_864);
    assert_eq!(pool.capacity_percent, 66);
    assert_eq!(pool.health, PoolHealth::Online);
    assert!(pool.is_online());
    assert!(!pool.read_only);
    let available = pool.available_percent().expect("pool has a size");
    assert!((available - 33.76).abs() < 0.01, "{available}");
}

#[test]
fn degraded_readonly_and_unknown_states_are_not_online() {
    let degraded = found(parse_pool("tank", "tank\t100\t90\t10\t90%\tDEGRADED\ton\n"));
    assert_eq!(degraded.health, PoolHealth::Degraded);
    assert!(!degraded.is_online());
    assert!(degraded.read_only);
    assert_eq!(degraded.capacity_percent, 90);
    let odd = found(parse_pool("tank", "tank\t100\t90\t10\t90\tWOBBLY\toff\n"));
    assert_eq!(odd.health, PoolHealth::Unrecognized("WOBBLY".into()));
    assert!(!odd.is_online());
    let suspended = found(parse_pool("tank", "tank\t100\t90\t10\t90\tSUSPENDED\toff"));
    assert_eq!(suspended.health, PoolHealth::Suspended);
}

#[test]
fn zero_size_pool_has_no_percentage() {
    let pool = found(parse_pool("tank", "tank\t0\t0\t0\t0\tFAULTED\toff\n"));
    assert_eq!(pool.available_percent(), None);
}

#[test]
fn garbage_is_unavailable_never_a_pool() {
    for text in [
        "",
        "\n",
        "not a pool at all",
        "zroot\t1\t2\t3\n",
        "zroot\tx\t1\t1\t1\tONLINE\toff\n",
        "zroot\t100\t1\t1\t101\tONLINE\toff\n",
        "zroot\t100\t1\t1\t-\tONLINE\toff\n",
        "zroot\t100\t200\t1\t1\tONLINE\toff\n",
        "zroot\t100\t1\t1\t1\tONLINE\tmaybe\n",
        "other\t100\t1\t1\t1\tONLINE\toff\n",
        "zroot\t100\t1\t1\t1\tONLINE\toff\nzroot\t100\t1\t1\t1\tONLINE\toff\n",
    ] {
        assert!(
            matches!(parse_pool("zroot", text), PoolObservation::Unavailable(_)),
            "{text:?}"
        );
    }
}

#[test]
fn missing_pool_is_distinct_from_other_failures() {
    assert_eq!(
        failure("cannot open 'nosuch': no such pool\n"),
        PoolObservation::NotFound
    );
    assert!(matches!(
        failure("internal error: failed to initialize ZFS library\n"),
        PoolObservation::Unavailable(_)
    ));
}

#[test]
fn dataset_names_resolve_to_their_pool() {
    assert_eq!(pool_of_dataset("zroot/ROOT/default"), Some("zroot"));
    assert_eq!(pool_of_dataset("zroot"), Some("zroot"));
    assert_eq!(pool_of_dataset("zroot/home@snap"), Some("zroot"));
    assert_eq!(pool_of_dataset(""), None);
    assert_eq!(pool_of_dataset("/dev/ada0"), None);
}

#[test]
fn hostile_pool_names_never_reach_a_command() {
    for name in ["", "-a", "-o", " zroot", "zr oot", "zroot\n", "1pool", "é"] {
        assert!(
            matches!(inspect_pool(name), PoolObservation::Unavailable(reason)
                if reason.starts_with("pool names")),
            "{name:?}"
        );
    }
}

#[cfg(not(target_os = "freebsd"))]
#[test]
fn other_platforms_answer_unavailable() {
    assert!(matches!(
        inspect_pool("zroot"),
        PoolObservation::Unavailable(_)
    ));
    assert!(matches!(
        inspect_pool_for_path(Path::new("/")),
        PoolObservation::Unavailable(_)
    ));
}

#[cfg(target_os = "freebsd")]
#[test]
fn live_pool_matches_its_backing_path() {
    let by_path = inspect_pool_for_path(Path::new("/"));
    if let PoolObservation::Found(pool) = by_path {
        assert_eq!(
            inspect_pool(&pool.name),
            PoolObservation::Found(pool.clone())
        );
        assert!(pool.size_bytes > 0);
    } else {
        // Not every FreeBSD host boots from ZFS.
        assert!(matches!(by_path, PoolObservation::Unavailable(_)));
    }
    assert_eq!(inspect_pool("vitalsnosuchpool"), PoolObservation::NotFound);
}
