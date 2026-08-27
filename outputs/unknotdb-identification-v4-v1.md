# Knot identification and invariant sidecar

- Graph snapshot: `outputs/unknotdb-rf-spherogram-expanded-v0.sqlite`
- Corpus representations: 3,519
- Representations mapped to current graph stopping keys: 3,476
- Representations with a bundled canonical knot name: 3,415
- Distinct named knots with determinant, Alexander and Jones data: 2,870
- Signature: not computed because Spherogram is unavailable in the existing RF environment.
- Sidecar: `outputs/unknotdb-identification-v4-v1.sqlite` (2,883,584 bytes)

This database is deliberately separate from the proof snapshot. Names, catalogue values and claimed tabular unknotting numbers are provenance-bearing metadata; they do not alter replay-validated `U_upper` or establish lower bounds. The Alexander polynomial is recomputed from each stored braid by RF Knots' exact invariant code. Determinant and Jones polynomial retain the bundled table provenance.
