# Anchored program-template analysis

- Snapshot: `outputs/unknotdb-rf-policy-v4-descending-v0.sqlite`
- Exact anchored templates: 15,277
- Semantic opcode families: 923
- Weighted contiguous subsequences (length 1–4): 906
- Independently verified reverse-endpoint edge pairs: 6,204

Families intentionally ignore numeric coordinates while retaining semantic operation order. They are search indexes, not new proof assertions. A reverse pair means that independently replayed stored edges connect the same two canonical vertices in opposite directions; it does not assert that their byte programs are formal syntactic inverses.

## Leading families

| Edge uses | Templates | Selected U | Family |
|---:|---:|---:|---|
| 50,007 | 11 | 34,972 | `CrossingChange` |
| 38,597 | 258 | 25,434 | `CrossingChange > NormalizeOrigin` |
| 9,336 | 27 | 7,684 | `CrossingChange > Reduce` |
| 6,156 | 317 | 2,803 | `Insert > CrossingChange > Reduce` |
| 5,044 | 144 | 2,749 | `CrossingChange > MirrorOrbit > NormalizeOrigin` |
| 4,672 | 2,047 | 851 | `Insert > CrossingChange > NormalizeOrigin > Reduce` |
| 4,330 | 226 | 3,404 | `CrossingChange > NormalizeOrigin > Reduce` |
| 4,255 | 269 | 3,827 | `CrossingChange > NormalizeOrigin > Reduce > NormalizeOrigin` |
| 1,713 | 184 | 444 | `Insert > DescendingCollapse` |
| 1,709 | 1,084 | 849 | `Insert > Insert > CrossingChange > Reduce > Reduce` |
| 1,560 | 391 | 638 | `Insert > CrossingChange > Reduce > Reduce` |
| 1,267 | 233 | 973 | `CrossingChange > MirrorOrbit > NormalizeOrigin > Reduce` |
| 1,039 | 819 | 236 | `Insert > Insert > CrossingChange > NormalizeOrigin > Reduce > Reduce` |
| 1,011 | 652 | 261 | `Insert > Insert > DescendingCollapse` |
| 941 | 77 | 933 | `CrossingChange > NormalizeOrigin > Reduce > Destabilize > NormalizeOrigin` |
| 614 | 391 | 193 | `Insert > CrossingChange > MirrorOrbit > NormalizeOrigin > Reduce` |
| 607 | 33 | 465 | `CrossingChange > Reduce > NormalizeOrigin` |
| 590 | 304 | 82 | `Insert > CrossingChange > NormalizeOrigin > Reduce > NormalizeOrigin` |
| 566 | 53 | 547 | `CrossingChange > NormalizeOrigin > Reduce > Reduce > NormalizeOrigin` |
| 562 | 371 | 153 | `Insert > CrossingChange > NormalizeOrigin > Reduce > Reduce` |

## Artifacts

- `outputs/unknotdb-template-analysis-v4-v0-families.tsv`
- `outputs/unknotdb-template-analysis-v4-v0-subsequences.tsv`
- `outputs/unknotdb-template-analysis-v4-v0-reverse-pairs.tsv`
- `outputs/unknotdb-template-analysis-v4-v0-utility.tsv`
