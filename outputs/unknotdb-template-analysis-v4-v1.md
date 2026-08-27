# Anchored program-template analysis

- Snapshot: `outputs/unknotdb-rf-spherogram-expanded-v0.sqlite`
- Exact anchored templates: 17,228
- Semantic opcode families: 1,046
- Weighted contiguous subsequences (length 1–4): 952
- Independently verified reverse-endpoint edge pairs: 6,216

Families intentionally ignore numeric coordinates while retaining semantic operation order. They are search indexes, not new proof assertions. A reverse pair means that independently replayed stored edges connect the same two canonical vertices in opposite directions; it does not assert that their byte programs are formal syntactic inverses.

## Leading families

| Edge uses | Templates | Selected U | Family |
|---:|---:|---:|---|
| 52,833 | 11 | 37,793 | `CrossingChange` |
| 39,009 | 316 | 25,830 | `CrossingChange > NormalizeOrigin` |
| 9,595 | 31 | 7,942 | `CrossingChange > Reduce` |
| 6,834 | 398 | 3,425 | `Insert > CrossingChange > Reduce` |
| 5,089 | 148 | 2,794 | `CrossingChange > MirrorOrbit > NormalizeOrigin` |
| 4,873 | 2,241 | 1,012 | `Insert > CrossingChange > NormalizeOrigin > Reduce` |
| 4,354 | 237 | 3,426 | `CrossingChange > NormalizeOrigin > Reduce` |
| 4,270 | 275 | 3,841 | `CrossingChange > NormalizeOrigin > Reduce > NormalizeOrigin` |
| 1,995 | 1,293 | 1,040 | `Insert > Insert > CrossingChange > Reduce > Reduce` |
| 1,818 | 192 | 530 | `Insert > DescendingCollapse` |
| 1,679 | 448 | 744 | `Insert > CrossingChange > Reduce > Reduce` |
| 1,287 | 248 | 993 | `CrossingChange > MirrorOrbit > NormalizeOrigin > Reduce` |
| 1,098 | 878 | 271 | `Insert > Insert > CrossingChange > NormalizeOrigin > Reduce > Reduce` |
| 1,090 | 694 | 309 | `Insert > Insert > DescendingCollapse` |
| 941 | 77 | 933 | `CrossingChange > NormalizeOrigin > Reduce > Destabilize > NormalizeOrigin` |
| 661 | 436 | 238 | `Insert > CrossingChange > MirrorOrbit > NormalizeOrigin > Reduce` |
| 652 | 622 | 305 | `Insert > Insert > Insert > CrossingChange > Reduce > Reduce > Reduce` |
| 642 | 1 | 618 | `DescendingCollapse` |
| 608 | 34 | 465 | `CrossingChange > Reduce > NormalizeOrigin` |
| 598 | 310 | 89 | `Insert > CrossingChange > NormalizeOrigin > Reduce > NormalizeOrigin` |

## Artifacts

- `outputs/unknotdb-template-analysis-v4-v1-families.tsv`
- `outputs/unknotdb-template-analysis-v4-v1-subsequences.tsv`
- `outputs/unknotdb-template-analysis-v4-v1-reverse-pairs.tsv`
- `outputs/unknotdb-template-analysis-v4-v1-utility.tsv`
