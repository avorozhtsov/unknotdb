# Catalogue U import and ASC10 discrepancy queue

- Graph snapshot: `outputs/unknotdb-rf-spherogram-expanded-v0.sqlite`
- Graph SHA-256: `53be2951728a0996ce094f28fc8b34397a04dd098910e27f5fee96ef27644257`
- Identification sidecar: `outputs/unknotdb-identification-maps-v3.sqlite`
- Identification SHA-256: `52b818c8a21664df5fafef146e2f701eea3839715ac8fe1de3ee1d413c1bfb43`
- Catalogue: `KnotInfo-2026-08-14` from `https://knotinfo.org/knotinfo_data_complete.xls`, retrieved `2026-08-14`
- Catalogue source SHA-256: `1829b7056eefc653f77a42acc9e471df4020e64fe93a8b6a211ca3d49fe86e7b`
- Parsed catalogue claims: 12,965; rejected nonempty values: 0
- Graph/name comparisons: 3,438
- Needs a shorter witness: 3,379
- Matches catalogue upper endpoint: 59
- Inside catalogue interval: 0
- Graph stronger than catalogue lower endpoint: 0
- Sidecar: `outputs/unknotdb-catalogue-u-v0.sqlite`
- Queue: `outputs/unknotdb-catalogue-u-asc10-queue-v0.tsv`

Catalogue claims are metadata only. They never alter `U_upper`, route pointers,
proof programs, or graph edges. Queue order is exactly `(ASC10, node_id, knot_id)`.

First actionable row: node `62796`, `knot:8_9`, ASC10 `48`, graph U upper `2`, catalogue `[1,1]`.
