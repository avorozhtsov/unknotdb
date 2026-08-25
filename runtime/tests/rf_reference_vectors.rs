use unknotdb_runtime::representation::{BraidRepresentation, SemanticAction};

const VECTORS: &str = include_str!("data/rf_reference_actions_v0.txt");

fn hex(value: &str) -> Vec<u8> {
    assert_eq!(value.len() % 2, 0);
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn rust_semantics_match_frozen_rf_reference_vectors() {
    let mut checked = 0;
    for line in VECTORS.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.split_whitespace().collect();
        assert_eq!(fields.len(), 3, "bad fixture line: {line}");
        let source = BraidRepresentation::decode(&hex(fields[0])).unwrap();
        let action = SemanticAction::decode_u63(fields[1].parse().unwrap()).unwrap();
        let expected = hex(fields[2]);
        let actual = action.apply(&source).unwrap().encode().unwrap();
        assert_eq!(actual, expected, "source={source:?} action={action:?}");
        checked += 1;
    }
    assert_eq!(checked, 256);
}
