# Jones PD backfill

- Parent sidecar: `outputs/unknotdb-lookup-maps-v1.sqlite`
- Parent SHA-256: `4cfdad53a1d9a9157f93956b378448ea72bf91dfa4170f5bfa64641b4b027a08`
- Output sidecar: `outputs/unknotdb-lookup-maps-v2.sqlite` (8,921,088 bytes)
- Output SHA-256: `f9fd663538c5a6d41641335b342894c05cc9e25e16bd30ed9221118269dfb6ae`
- Missing Jones values before: 531
- Missing Jones values after: 0
- Known values independently replayed: 2,870
- Crossing-count distribution: `{"11": 4, "12": 101, "13": 426}`
- Algorithm: `pd-kauffman-state-sum-v1`
- SQLite integrity: `ok`

Every source braid was checked for exact equality with the braid generated from the same pinned Spherogram named diagram. Each polynomial passed `V(1)=1` and `abs(V(-1))=det(K)`. The computation enumerates the minimal PD diagram's `2^crossings` Kauffman states rather than the wide braid's Temperley-Lieb basis.
