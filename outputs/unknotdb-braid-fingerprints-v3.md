# Braid and representation fingerprints v2

- Sidecar: `outputs/unknotdb-braid-fingerprints-v3.sqlite` (11,341,824 bytes)
- Representations: 3,519
- Fingerprint definitions: 14
- Stored postings: 49,134
- Graph snapshot SHA-256: `53be2951728a0996ce094f28fc8b34397a04dd098910e27f5fee96ef27644257`
- Identification maps SHA-256: `52b818c8a21664df5fafef146e2f701eea3839715ac8fe1de3ee1d413c1bfb43`
- Fingerprint sidecar SHA-256: `e970c6fd3846dce03204ab49ced4c67d26c3b9cd18a1d41a0e9d196c6f26c542`
- SQLite integrity: `ok`

Every definition records `scope`, `valid_under`, and `safe_use`. None of these values is admitted as a general knot invariant. Reverse lookup intersects the indexed postings `(fingerprint_id,value_text)` and is intended for candidate generation, bucketing and ranking only.
