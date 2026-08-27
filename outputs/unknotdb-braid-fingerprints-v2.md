# Braid and representation fingerprints v1

- Sidecar: `outputs/unknotdb-braid-fingerprints-v2.sqlite` (11,317,248 bytes)
- Representations: 3,519
- Fingerprint definitions: 14
- Stored postings: 49,134
- Graph snapshot SHA-256: `53be2951728a0996ce094f28fc8b34397a04dd098910e27f5fee96ef27644257`
- Lookup maps SHA-256: `f9fd663538c5a6d41641335b342894c05cc9e25e16bd30ed9221118269dfb6ae`
- Fingerprint sidecar SHA-256: `97a33c87e1c494b91c16fb2b632abf9cf86550575d52077317cd66c8430fd4f1`
- SQLite integrity: `ok`

Every definition records `scope`, `valid_under`, and `safe_use`. None of these values is admitted as a general knot invariant. Reverse lookup intersects the indexed postings `(fingerprint_id,value_text)` and is intended for candidate generation, bucketing and ranking only.
