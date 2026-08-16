//! Consume the crate exactly as an external caller would. This is the test that
//! actually proves the `[lib]` target is usable; the unit tests would pass just
//! as well from inside a binary-only crate.

use unknotdb::cert::Cert;
use unknotdb::diagram::Diagram;
use unknotdb::moves::Move;
use unknotdb::search::unknotting_trace;

#[test]
fn build_a_diagram_and_query_it() {
    let trefoil = Diagram::from_braid(2, &[1, 1, 1]).unwrap();
    assert_eq!(trefoil.n, 3);
    assert_eq!(trefoil.writhe(), 3);
    assert_eq!(trefoil.components(), 1);
    assert_eq!(
        trefoil.canon(true),
        Diagram::from_pd(&trefoil.to_pd()).unwrap().canon(true)
    );
}

#[test]
fn apply_moves_through_the_public_api() {
    let k = Diagram::from_braid(2, &[1, 1, 1]).unwrap();
    let unknotted = k.apply(Move::Xc { c: 0 }).unwrap();
    let (reduced, _) = unknotted.reduce();
    assert_eq!(reduced.n, 0);
}

#[test]
fn produce_and_verify_a_certificate_end_to_end() {
    let k = Diagram::from_braid(2, &[1, 1, 1, 1, 1]).unwrap(); // 5_1, u = 2
    let (cost, trace) = unknotting_trace(&k, 3, 100_000, 2).expect("trace");
    assert_eq!(cost, 2);
    let text = unknotdb::cert::emit(
        "unknotting_number_le",
        cost,
        &k,
        &trace,
        Some("5_1"),
        "Rolfsen 1976",
        "integration test",
        "2026-08-16",
    );
    let report = Cert::parse(&text)
        .and_then(|c| c.verify())
        .expect("verifies");
    assert_eq!(report.cost, 2);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn a_tampered_certificate_is_rejected() {
    let k = Diagram::from_braid(2, &[1, 1, 1]).unwrap();
    let (cost, trace) = unknotting_trace(&k, 2, 100_000, 2).unwrap();
    let good = unknotdb::cert::emit(
        "unknotting_number_le",
        cost,
        &k,
        &trace,
        None,
        "s",
        "a",
        "2026-08-16",
    );
    assert!(Cert::parse(&good).and_then(|c| c.verify()).is_ok());
    // Claim a bound the trace does not support.
    let bad = good.replace("value: 1", "value: 0");
    assert!(Cert::parse(&bad).and_then(|c| c.verify()).is_err());
}
