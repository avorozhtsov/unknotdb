# Lower-bound sidecar v0

This sidecar stores cited or independently recomputed lower bounds separately
from the immutable upper-bound proof graph. A lower bound is never represented
as a crossing-change edge.

`lower_bound_claims` contains one effective interval per source and knot.
`claim_evidence` is the many-to-many link to method-specific certificate rows.
`sources` pins the repository commit and tree, names the associated work, and
links its citation and erratum files. `source_artifacts` pins every imported
file by SHA-256. Reverse lookup is indexed by `(u_lower, knot_id)`.
The read-only `catalogue_u_claims` compatibility view lets existing bounded
catalogue-witness generators consume exact DGKT intervals without copying or
reinterpreting them.

Trust statuses are deliberately narrow:

- `externally_attested`: the cited source asserts the mathematical claim; the
  importer checked hashes, schema, interval arithmetic, correction policy and
  the existence of every supporting certificate row.
- `verified`: an independent Unknot DB verifier recomputed the mathematical
  certificate from pinned inputs. Merely running a source-provided consistency
  checker does not qualify.

An interval with equal lower and retained upper is an exact value only relative
to both cited ingredients: the new lower-bound certificate and the pinned
catalogue upper bound. The sidecar does not alter `U_upper`, graph routing or
the status of any proof edge.

The DGKT v0 importer rejects dirty source checkouts, unknown methods, duplicate
knots, missing method-specific certificate rows and non-improving intervals.
It excludes the withdrawn Owens batches by importing only
`lower_bounds/owens/valid_determinant_group_certificates.csv` from a commit
whose `ERRATUM.md` is hash-pinned.
