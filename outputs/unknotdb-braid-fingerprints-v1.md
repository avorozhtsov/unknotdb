# Braid and representation fingerprints v1

- Sidecar: `outputs/unknotdb-braid-fingerprints-v1.sqlite` (11,317,248 bytes)
- Representations: 3,519
- Fingerprint definitions: 14
- Stored postings: 49,134
- Deduplicated typed values: 9,861
- Graph snapshot SHA-256: `53be2951728a0996ce094f28fc8b34397a04dd098910e27f5fee96ef27644257`
- Lookup maps SHA-256: `4cfdad53a1d9a9157f93956b378448ea72bf91dfa4170f5bfa64641b4b027a08`
- Fingerprint sidecar SHA-256: `be2d34a0b451fe0766919ad05f988387e88c01eadd6c08f7319d4f015649c027`
- SQLite integrity: `ok`
- Warm three-fingerprint intersection: about 22 microseconds on this machine
- Dictionary normalization reduced the sidecar from 15,470,592 to 11,317,248 bytes (26.8%)

Every definition records `scope`, `valid_under`, and `safe_use`. None of these values is admitted as a general knot invariant. Reverse lookup intersects the indexed postings `(fingerprint_id,value_text)` and is intended for candidate generation, bucketing and ranking only.
