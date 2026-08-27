# Catalogue U import and ASC10 discrepancy queue

- Graph snapshot: `outputs/unknotdb-10_48-u2-v0.sqlite`
- Graph SHA-256: `ec1d25c96a34932ec2d5faab53939d217b7698d83253f96f3029568621707f36`
- Identification sidecar: `outputs/unknotdb-identification-maps-v3.sqlite`
- Identification SHA-256: `52b818c8a21664df5fafef146e2f701eea3839715ac8fe1de3ee1d413c1bfb43`
- Catalogue: `KnotInfo-2026-08-14` from `https://knotinfo.org/knotinfo_data_complete.xls`, retrieved `2026-08-14`
- Catalogue source SHA-256: `1829b7056eefc653f77a42acc9e471df4020e64fe93a8b6a211ca3d49fe86e7b`
- Parsed catalogue claims: 12,965; rejected nonempty values: 0
- Graph/name comparisons: 3,438
- Needs a shorter witness: 3,375
- Matches catalogue upper endpoint: 63
- Inside catalogue interval: 0
- Graph stronger than catalogue lower endpoint: 0
- Sidecar: `outputs/unknotdb-catalogue-u-v2.sqlite`
- Queue: `outputs/unknotdb-catalogue-u-asc10-queue-v2.tsv`

Catalogue claims are metadata only. They never alter `U_upper`, route pointers,
proof programs, or graph edges. Queue order is exactly `(ASC10, node_id, knot_id)`.

First actionable row: node `47951`, `knot:10_141`, ASC10 `50`, graph U upper `2`, catalogue `[1,1]`.
