//! Knot diagrams as combinatorial maps.
//!
//! A diagram with `n` crossings has `4n` darts. Dart `4c + p` is position `p`
//! of crossing `c`, counted counterclockwise. Positions 0 and 2 carry the
//! under-strand, 1 and 3 the over-strand; this invariant is maintained by every
//! move. `sigma` rotates counterclockwise about a crossing, `alpha` pairs the
//! two ends of an arc. See `schema/conventions.md`.

use std::collections::HashMap;

pub type Dart = usize;

#[inline]
pub fn cr(d: Dart) -> usize {
    d >> 2
}
#[inline]
pub fn pos(d: Dart) -> usize {
    d & 3
}
#[inline]
pub fn dart(c: usize, p: usize) -> Dart {
    (c << 2) | (p & 3)
}
#[inline]
pub fn sigma(d: Dart) -> Dart {
    dart(cr(d), pos(d) + 1)
}
#[inline]
pub fn sigma2(d: Dart) -> Dart {
    dart(cr(d), pos(d) + 2)
}
/// Positions 1 and 3 are the over-strand.
#[inline]
pub fn is_over(d: Dart) -> bool {
    pos(d) & 1 == 1
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagram {
    pub n: usize,
    pub alpha: Vec<Dart>,
    /// Crossingless circles produced by splicing. A knot diagram that reduces
    /// to the unknot has `n == 0` and `free_loops <= 1`.
    pub free_loops: usize,
}

impl Diagram {
    pub fn empty() -> Diagram {
        Diagram {
            n: 0,
            alpha: vec![],
            free_loops: 1,
        }
    }

    /// Permissive PD reader: accepts `PD[X[1,4,2,5],...]`, `[[1,4,2,5],...]`,
    /// or a bare list of integers. Arc labels may start at 0 or 1.
    pub fn from_pd(s: &str) -> Result<Diagram, String> {
        let mut nums: Vec<i64> = Vec::new();
        let mut cur = String::new();
        for ch in s.chars() {
            if ch.is_ascii_digit() || (ch == '-' && cur.is_empty()) {
                cur.push(ch);
            } else if !cur.is_empty() {
                nums.push(cur.parse::<i64>().map_err(|e| e.to_string())?);
                cur.clear();
            }
        }
        if !cur.is_empty() {
            nums.push(cur.parse::<i64>().map_err(|e| e.to_string())?);
        }
        if nums.is_empty() {
            return Ok(Diagram::empty());
        }
        if nums.len() % 4 != 0 {
            return Err(format!(
                "PD: {} integers is not a multiple of 4",
                nums.len()
            ));
        }
        let n = nums.len() / 4;

        let mut occ: HashMap<i64, Vec<Dart>> = HashMap::new();
        for (i, &label) in nums.iter().enumerate() {
            occ.entry(label).or_default().push(i);
        }
        let mut alpha = vec![usize::MAX; 4 * n];
        for (label, ds) in &occ {
            if ds.len() != 2 {
                return Err(format!(
                    "PD: arc label {} occurs {} times, expected exactly 2",
                    label,
                    ds.len()
                ));
            }
            alpha[ds[0]] = ds[1];
            alpha[ds[1]] = ds[0];
        }
        let d = Diagram {
            n,
            alpha,
            free_loops: 0,
        };
        d.validate()?;
        Ok(d)
    }

    /// Closure of a braid word given as signed Artin generators, `+j` for
    /// `sigma_j` and `-j` for its inverse, on `strands` strands.
    ///
    /// The braid runs top to bottom. Around a crossing the counterclockwise
    /// order of ends is top-left, bottom-left, bottom-right, top-right; for
    /// `sigma_j` the strand entering top-left passes UNDER, so that a positive
    /// generator yields a positive crossing under the sign convention of
    /// `schema/conventions.md`. Dart positions are
    /// then rotated so that the under-strand sits at positions 0 and 2, which
    /// is the invariant the rest of the crate relies on.
    /// `B_1` is legal and is the unknot: it has no generators, so the word must
    /// be empty and the closure is a single circle. Real corpora do contain it
    /// — rf-knots seeds unknot rungs that way — so rejecting it is wrong.
    pub fn from_braid(strands: usize, word: &[i32]) -> Result<Diagram, String> {
        if strands < 1 {
            return Err("braid: need at least 1 strand".into());
        }
        for &g in word {
            let j = g.unsigned_abs() as usize;
            if g == 0 || j >= strands {
                return Err(format!(
                    "braid: generator {} out of range for {} strands",
                    g, strands
                ));
            }
        }
        let n = word.len();
        if n == 0 {
            return Ok(Diagram {
                n: 0,
                alpha: vec![],
                free_loops: strands,
            });
        }
        let mut alpha = vec![usize::MAX; 4 * n];
        let mut top: Vec<Option<Dart>> = vec![None; strands];
        let mut cur: Vec<Option<Dart>> = vec![None; strands];
        let link = |alpha: &mut Vec<Dart>, a: Dart, b: Dart| {
            alpha[a] = b;
            alpha[b] = a;
        };

        for (c, &g) in word.iter().enumerate() {
            let j = g.unsigned_abs() as usize - 1; // left strand index, 0-based
                                                   // (top-left, bottom-left, bottom-right, top-right)
            let (tl, bl, br, tr) = if g > 0 {
                (dart(c, 0), dart(c, 1), dart(c, 2), dart(c, 3))
            } else {
                (dart(c, 3), dart(c, 0), dart(c, 1), dart(c, 2))
            };
            for (slot, up) in [(j, tl), (j + 1, tr)] {
                match cur[slot] {
                    None => top[slot] = Some(up),
                    Some(prev) => link(&mut alpha, prev, up),
                }
            }
            cur[j] = Some(bl);
            cur[j + 1] = Some(br);
        }

        let mut free = 0;
        for i in 0..strands {
            match (cur[i], top[i]) {
                (Some(a), Some(b)) => link(&mut alpha, a, b),
                (None, None) => free += 1, // strand untouched by any generator
                _ => return Err("braid: inconsistent closure".into()),
            }
        }
        let d = Diagram {
            n,
            alpha,
            free_loops: free,
        };
        d.validate()?;
        Ok(d)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.alpha.len() != 4 * self.n {
            return Err(format!(
                "alpha has {} entries, expected {}",
                self.alpha.len(),
                4 * self.n
            ));
        }
        for d in 0..4 * self.n {
            let a = self.alpha[d];
            if a >= 4 * self.n {
                return Err(format!("dart {} maps out of range", d));
            }
            if a == d {
                return Err(format!("dart {} is paired with itself", d));
            }
            if self.alpha[a] != d {
                return Err(format!("alpha is not an involution at dart {}", d));
            }
        }
        if self.n > 0 {
            // Euler characteristic of the sphere: V - E + F = 2, with V = n,
            // E = 2n. Any non-planar or inconsistent PD fails here.
            let f = self.faces().len();
            if f != self.n + 2 {
                return Err(format!(
                    "not a planar diagram: {} faces, expected {} (V-E+F != 2)",
                    f,
                    self.n + 2
                ));
            }
        }
        Ok(())
    }

    /// Faces are the orbits of `phi(d) = sigma(alpha(d))`.
    pub fn faces(&self) -> Vec<Vec<Dart>> {
        let mut seen = vec![false; 4 * self.n];
        let mut out = Vec::new();
        for start in 0..4 * self.n {
            if seen[start] {
                continue;
            }
            let mut orbit = Vec::new();
            let mut d = start;
            loop {
                seen[d] = true;
                orbit.push(d);
                d = sigma(self.alpha[d]);
                if d == start {
                    break;
                }
            }
            out.push(orbit);
        }
        out
    }

    /// Number of link components. A knot has exactly 1.
    pub fn components(&self) -> usize {
        if self.n == 0 {
            return self.free_loops;
        }
        let mut seen = vec![false; 4 * self.n];
        let mut orbits = 0;
        for start in 0..4 * self.n {
            if seen[start] {
                continue;
            }
            orbits += 1;
            let mut d = start;
            loop {
                seen[d] = true;
                d = self.alpha[sigma2(d)];
                if d == start {
                    break;
                }
            }
        }
        // Incoming and outgoing darts form separate orbits of the same strand.
        orbits / 2 + self.free_loops
    }

    /// `true` at darts where the strand enters its crossing. The global choice
    /// of direction is arbitrary; crossing signs do not depend on it.
    pub fn orientation(&self) -> Vec<bool> {
        let mut incoming = vec![false; 4 * self.n];
        let mut seen = vec![false; 4 * self.n];
        for start in 0..4 * self.n {
            if seen[start] {
                continue;
            }
            let mut d = start;
            loop {
                if seen[d] {
                    break;
                }
                seen[d] = true;
                incoming[d] = true;
                let out = sigma2(d);
                seen[out] = true;
                incoming[out] = false;
                d = self.alpha[out];
            }
        }
        incoming
    }

    /// Crossing signs under the convention in `schema/conventions.md`:
    /// with the incoming under-strand at position `q`, the crossing is positive
    /// iff the incoming over-strand is at position `q + 3`.
    pub fn signs(&self) -> Vec<i8> {
        let inc = self.orientation();
        (0..self.n)
            .map(|c| {
                let q = if inc[dart(c, 0)] { 0 } else { 2 };
                if inc[dart(c, q + 3)] {
                    1
                } else {
                    -1
                }
            })
            .collect()
    }

    pub fn writhe(&self) -> i64 {
        self.signs().iter().map(|&s| s as i64).sum()
    }

    /// Signed Gauss code read from `start`, forwards or backwards.
    fn gauss_from(&self, start: Dart, forward: bool, signs: &[i8]) -> String {
        let mut order: HashMap<usize, usize> = HashMap::new();
        let mut out = String::new();
        let mut d = start;
        for _ in 0..2 * self.n {
            let c = cr(d);
            let next_id = order.len() + 1;
            let id = *order.entry(c).or_insert(next_id);
            out.push(if is_over(d) { 'O' } else { 'U' });
            out.push_str(&id.to_string());
            out.push(if signs[c] > 0 { '+' } else { '-' });
            d = if forward {
                self.alpha[sigma2(d)]
            } else {
                sigma2(self.alpha[d])
            };
        }
        out
    }

    /// Lexicographically minimal signed Gauss code over all starting darts and
    /// both traversal directions, and both mirrors when `mirror_invariant`.
    /// This is the primary key. See `schema/conventions.md` §3.
    pub fn canon(&self, mirror_invariant: bool) -> String {
        if self.n == 0 {
            return "U".to_string();
        }
        let signs = self.signs();
        let mut best: Option<String> = None;
        let mut consider = |s: String| {
            if best.as_ref().map_or(true, |b| s < *b) {
                best = Some(s);
            }
        };
        for start in 0..4 * self.n {
            consider(self.gauss_from(start, true, &signs));
            consider(self.gauss_from(start, false, &signs));
        }
        if mirror_invariant {
            let flipped: Vec<i8> = signs.iter().map(|s| -s).collect();
            let m = self.mirror();
            for start in 0..4 * self.n {
                consider(m.gauss_from(start, true, &flipped));
                consider(m.gauss_from(start, false, &flipped));
            }
        }
        best.unwrap()
    }

    /// Reflect every crossing: over becomes under everywhere.
    pub fn mirror(&self) -> Diagram {
        let mut d = self.clone();
        for c in 0..self.n {
            d = d.crossing_change(c).expect("mirror: crossing exists");
        }
        d
    }

    /// PD code with arc labels assigned along the strand, each crossing rotated
    /// so that position 0 is the incoming under-strand (the PD convention).
    pub fn to_pd(&self) -> String {
        if self.n == 0 {
            return "PD[]".to_string();
        }
        let inc = self.orientation();
        let mut label = vec![0usize; 4 * self.n];
        let mut next = 1usize;
        let mut seen = vec![false; 4 * self.n];
        for start in 0..4 * self.n {
            if seen[start] || !inc[start] {
                continue;
            }
            let mut d = start;
            loop {
                if seen[d] {
                    break;
                }
                seen[d] = true;
                let out = sigma2(d);
                seen[out] = true;
                let nxt = self.alpha[out];
                label[out] = next;
                label[nxt] = next;
                next += 1;
                d = nxt;
            }
        }
        let parts: Vec<String> = (0..self.n)
            .map(|c| {
                let q = if inc[dart(c, 0)] { 0 } else { 2 };
                let l: Vec<String> = (0..4).map(|k| label[dart(c, q + k)].to_string()).collect();
                format!("X[{}]", l.join(","))
            })
            .collect();
        format!("PD[{}]", parts.join(", "))
    }
}
