# Identification and unknotting audit of the final seven high-U vertices

Date: 2026-08-26. Source snapshot: `outputs/unknotdb-branch-depth5-ge8-v0.sqlite`.

This report separates knot identification, lower bounds, and explicit upper-bound
witnesses. Crossing positions are one-based positions in the exact braid word printed
in the TSV companion. A crossing change negates the corresponding Artin generator.

## Result

The seven vertices represent six knot types: two different stopping points are mirrors
of `10_126`. Five types are identified by exact Jones/Alexander fingerprints and
SnapPy complement matching. The two remaining complements simplify to respectively
15- and 16-crossing hyperbolic diagrams and are not present in the local KnotInfo table
through 13 crossings; they are recorded by their exact braid words and fingerprints.

| key prefix | identification | old DB U | mathematical result | explicit CC witness |
|---|---:|---:|---:|---:|
| `0710f715` | unnamed 15-crossing hyperbolic knot | 9 | `u=3` | positions 7,8,13 |
| `4dc995f3` | mirror of `12n_304` | 9 | only `1 <= u <= 3` is known | positions 3,4,9 prove `u<=3` |
| `15159234` | unnamed 16-crossing hyperbolic knot | 8 | `u=4` | positions 4,5,6,8 |
| `2f774de6` | mirror of `10_126` | 8 | `u=2` | positions 3,4 |
| `b24614e4` | `12n_166` | 8 | `u=4` | positions 3,4,9,14 |
| `6bcf8db3` | mirror of `13n_259` | 8 | `u=3` | public tabulated upper; no replayable crossing list located |
| `b11500db` | mirror of `10_126` | 8 | `u=2` | positions 3,4 |

Thus six vertices can immediately fall below 8 once these witnesses are compiled into
Unknot DB proof programs. The remaining `12n_304` vertex can fall from 9 to 3, but must
remain labelled as an upper bound, not an exact value.

## Exact invariants

Alexander coefficient lists are ordered from exponent zero upward, after multiplication
by a unit. Mirror images have the same Alexander polynomial and opposite Jones exponents,
signature, and tau.

| key | Alexander coefficients | det | Jones coefficients by exponent | signature | tau |
|---|---|---:|---|---:|---:|
| `0710f715` | `1,-4,10,-15,18,-19,18,-15,10,-4,1` | 115 | `2:-1,3:4,4:-7,5:12,6:-15,7:18,8:-18,9:16,10:-12,11:7,12:-4,13:1` | -6 | 3 |
| `4dc995f3` | `3,-9,15,-17,15,-9,3` | 71 | `-2:-2,-1:4,0:-6,1:10,2:-11,3:12,4:-10,5:8,6:-5,7:2,8:-1` | -2 | 1 |
| `15159234` | `1,-2,4,-7,10,-11,10,-7,4,-2,1` | 59 | `5:2,6:-2,7:5,8:-7,9:8,10:-9,11:8,12:-7,13:5,14:-3,15:2,16:-1` | -6 | 4 |
| `2f774de6` | `1,-2,4,-5,4,-2,1` | 19 | `0:-1,1:2,2:-2,3:4,4:-3,5:3,6:-2,7:1,8:-1` | -2 | 1 |
| `b24614e4` | `2,-4,5,-6,7,-6,5,-4,2` | 41 | `4:1,5:-1,6:3,7:-4,8:6,9:-6,10:6,11:-6,12:4,13:-3,14:1` | -8 | 4 |
| `6bcf8db3` | `1,-3,5,-4,2,-1,2,-4,5,-3,1` | 31 | `0:-1,1:2,2:-3,3:5,4:-4,5:5,6:-4,7:3,8:-2,9:1,10:-1` | -6 | 3 |
| `b11500db` | same as `2f774de6` | 19 | same as `2f774de6` | -2 | 1 |

For `0710f715` and `15159234`, `|tau| <= u` gives lower bounds 3 and 4; the
listed replayed CC sets give matching upper bounds. For `12n_166`, the signature gives
`u >= 4` and the four-CC witness gives `u <= 4`. For mirror `13n_259`, the signature
gives `u >= 3` and KnotInfo tabulates `u=3`. For `10_126`, the two-CC upper witness is
paired with the published Heegaard-Floer obstruction to unknotting number one cited by
KnotInfo.

## Witness validation and limitations

Each printed changed braid was converted independently to a planar-diagram code and
loaded into Regina 7.4. In every populated witness row,
`Link.complement().isSolidTorus()` returned true. This establishes that the changed
closure is an unknot without trusting Spherogram's simplifier. The exact words and
positions are in `unknotdb-last-seven-knot-identification-20260826-v0.tsv`.

The public sources supply tabulated values, braid representatives, and theoretical lower
bounds, but generally do not publish machine-replayable crossing indices. The explicit
crossing lists above were found locally from the supplied Unknot DB representatives and
then independently replayed. No public machine-replayable three-crossing witness for
`13n_259` was located. No source found after the 2026 KnotInfo data resolves the
`12n_304` interval; reporting an exact value for it would be a new mathematical claim.

The two unnamed knots have unverified-by-Sage SnapPy isometry signatures:

* `0710f715`: `yLLPwvzLLALALQQMQccdfehjnolsntwtvvsuwuxuxxqffaavcagcxcxaxpmaagcevui`, volume about 16.434178679.
* `15159234`: `rvLAvMvQMzQQccdfiglmjknopnopqqqffaafdabbdabfkbkbj`, volume about 13.000896428.

These signatures are useful database fingerprints, not substitutes for the exact braid
word or a certified census-name match.

## Provenance

* KnotInfo complete-data mirror used locally:
  `rf-knots/tmp/knotinfo_data_complete-2026-08-14.xls`, SHA-256
  `1829b7056eefc653f77a42acc9e471df4020e64fe93a8b6a211ca3d49fe86e7b`.
* Knot identity: exact local Alexander/Jones computations plus complement matching;
  mirrors were resolved by Jones exponent reversal and signature/tau sign.
* Terminal check: Spherogram 2.4.1 PD export, Regina 7.4 solid-torus recognition.
* No policy network, training, RF mutation, database ingestion, or commit was performed
  by this identification audit.
