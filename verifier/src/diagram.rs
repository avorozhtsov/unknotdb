//! Knot diagrams as combinatorial maps.
//!
//! A diagram with `n` crossings has `4n` darts. Dart `4c + p` is position `p`
//! of crossing `c`, counted counterclockwise. Positions 0 and 2 carry the
//! under-strand, 1 and 3 the over-strand; this invariant is maintained by every
//! move. `sigma` rotates counterclockwise about a crossing, `alpha` pairs the
//! two ends of an arc. See `schema/conventions.md`.

use std::collections::{HashMap, HashSet};

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

    /// Compile a *cyclic* braid word — the natural language for braids drawn on
    /// a cylinder, where the closure lives on a torus — into an ordinary Artin
    /// word.
    ///
    /// Generators are indexed mod `strands`: `±strands` is the seam band that
    /// joins strand `strands` back to strand 1 across the cylinder's seam. This
    /// is not new topology. The seam generator is the Birman–Ko–Lee band
    /// `a_{1,n} = w sigma_1 w^-1` with `w = sigma_{n-1} ... sigma_2`, an
    /// ordinary element of `B_n`, so a cyclic word denotes an ordinary braid
    /// and closes to an ordinary link with a planar diagram.
    ///
    /// The cost semantics survive: exactly one letter of the expansion is the
    /// band's own crossing, and the conjugating letters cancel in pairs. So
    /// flipping one seam generator is still exactly one crossing change, and a
    /// cyclic unknotting path compiles to a planar one of the same `u`.
    ///
    /// The point of the cylinder is that it gives more single-crossing-change
    /// options per state — any pair of strands, not just adjacent ones — which
    /// is why unknotting paths are often shorter there.
    pub fn compile_cyclic(strands: usize, word: &[i32]) -> Result<Vec<i32>, String> {
        if strands < 2 {
            if word.is_empty() {
                return Ok(vec![]);
            }
            return Err("cyclic braid: a one-strand braid cannot contain generators".into());
        }
        let prefix: Vec<i32> = (2..strands as i32).rev().collect();
        let mut out = Vec::new();
        for &g in word {
            let j = g.unsigned_abs() as usize;
            if g == 0 || j > strands {
                return Err(format!(
                    "cyclic braid: generator {} is invalid for B*_{}",
                    g, strands
                ));
            }
            if j == strands {
                out.extend(prefix.iter().copied());
                out.push(if g > 0 { 1 } else { -1 });
                out.extend(prefix.iter().rev().map(|v| -v));
            } else {
                out.push(g);
            }
        }
        Ok(out)
    }

    /// Closure of a cyclic braid word. See [`Diagram::compile_cyclic`].
    pub fn from_cyclic_braid(strands: usize, word: &[i32]) -> Result<Diagram, String> {
        Diagram::from_braid(strands, &Diagram::compile_cyclic(strands, word)?)
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
            // Euler characteristic of the sphere, V - E + F = 2, holds per
            // connected component, with V = n and E = 2n overall. A split
            // diagram — the closure of sigma_1 sigma_3 in B_4, say — is two
            // disjoint kinks and genuinely has n + 2k faces, so requiring
            // n + 2 would reject perfectly good input.
            let k = self.map_components();
            let f = self.faces().len();
            if f != self.n + 2 * k {
                return Err(format!(
                    "not a planar diagram: {} faces over {} connected component(s), \
                     expected {} (V-E+F != 2 per component)",
                    f,
                    k,
                    self.n + 2 * k
                ));
            }
        }
        Ok(())
    }

    /// Connected components of the underlying 4-valent graph, i.e. orbits of
    /// the group generated by `sigma` and `alpha`. A split diagram has more
    /// than one; this is *not* the number of link components.
    pub fn map_components(&self) -> usize {
        let mut seen = vec![false; 4 * self.n];
        let mut count = 0;
        for start in 0..4 * self.n {
            if seen[start] {
                continue;
            }
            count += 1;
            let mut stack = vec![start];
            while let Some(d) = stack.pop() {
                if seen[d] {
                    continue;
                }
                seen[d] = true;
                stack.push(sigma(d));
                stack.push(self.alpha[d]);
            }
        }
        count
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

    /// Link components: orbits of the darts under `alpha` and `sigma^2`, i.e.
    /// "same strand". Distinct from [`Diagram::map_components`], which is about
    /// connectedness of the diagram.
    fn dart_components(&self) -> (Vec<usize>, usize) {
        let mut comp = vec![usize::MAX; 4 * self.n];
        let mut k = 0;
        for s in 0..4 * self.n {
            if comp[s] != usize::MAX {
                continue;
            }
            let mut stack = vec![s];
            while let Some(d) = stack.pop() {
                if comp[d] != usize::MAX {
                    continue;
                }
                comp[d] = k;
                stack.push(self.alpha[d]);
                stack.push(sigma2(d));
            }
            k += 1;
        }
        (comp, k)
    }

    /// One component's signed Gauss code, walked from `start` until it closes.
    /// Crossing ids come from `ids`, numbered by order of first visit across
    /// the whole diagram, so a crossing shared with an earlier component keeps
    /// the id it was already given.
    fn segment(
        &self,
        start: Dart,
        forward: bool,
        signs: &[i8],
        ids: &mut HashMap<usize, usize>,
    ) -> String {
        let mut out = String::new();
        let mut d = start;
        loop {
            let c = cr(d);
            let next_id = ids.len() + 1;
            let id = *ids.entry(c).or_insert(next_id);
            out.push(if is_over(d) { 'O' } else { 'U' });
            out.push_str(&id.to_string());
            out.push(if signs[c] > 0 { '+' } else { '-' });
            d = if forward {
                self.alpha[sigma2(d)]
            } else {
                sigma2(self.alpha[d])
            };
            if d == start {
                break;
            }
        }
        out
    }

    /// Canonical code for this diagram with its own handedness.
    ///
    /// Minimised over every starting dart, both traversal directions, and —
    /// for a link — every order in which the components may be listed, with
    /// `|` between components. Because crossing ids depend on the order of
    /// first visit, and a crossing shared between two components is numbered by
    /// whichever is walked first, the per-component codes are not independent
    /// and the minimisation has to be joint.
    ///
    /// Done by keeping a frontier of exactly those partial codes that tie for
    /// the minimum, which collapses to the diagram's symmetry group rather than
    /// enumerating `c!` orderings.
    fn canon_oriented(&self, signs: &[i8]) -> String {
        if self.n == 0 {
            return vec!["U"; self.free_loops].join("|");
        }
        let (comp, ncomp) = self.dart_components();
        let mut frontier: Vec<(String, HashMap<usize, usize>, Vec<bool>)> =
            vec![(String::new(), HashMap::new(), vec![false; ncomp])];

        for round in 0..ncomp {
            let mut best: Option<String> = None;
            let mut next: Vec<(String, HashMap<usize, usize>, Vec<bool>)> = Vec::new();
            let mut seen: HashSet<String> = HashSet::new();
            for (code, ids, used) in &frontier {
                for d in 0..4 * self.n {
                    if used[comp[d]] {
                        continue;
                    }
                    for &forward in &[true, false] {
                        let mut ids2 = ids.clone();
                        let seg = self.segment(d, forward, signs, &mut ids2);
                        let mut code2 = String::with_capacity(code.len() + seg.len() + 1);
                        code2.push_str(code);
                        if round > 0 {
                            code2.push('|');
                        }
                        code2.push_str(&seg);
                        let better = match &best {
                            None => true,
                            Some(b) => code2 < *b,
                        };
                        if better {
                            best = Some(code2.clone());
                            next.clear();
                            seen.clear();
                        } else if Some(&code2) != best.as_ref() {
                            continue;
                        }
                        let mut used2 = used.clone();
                        used2[comp[d]] = true;
                        let mut key: Vec<(usize, usize)> =
                            ids2.iter().map(|(a, b)| (*a, *b)).collect();
                        key.sort_unstable();
                        let key = format!("{:?}{:?}", key, used2);
                        if seen.insert(key) {
                            next.push((code2, ids2, used2));
                        }
                    }
                }
            }
            frontier = next;
        }
        let mut code = frontier
            .into_iter()
            .next()
            .map(|(code, _, _)| code)
            .unwrap_or_default();
        // Crossingless circles live outside the dart structure, so they have to
        // be appended explicitly: a knot and that knot split off from a free
        // circle are different links and must not share a key.
        for _ in 0..self.free_loops {
            code.push_str("|U");
        }
        code
    }

    /// Crossing signs under a given choice of direction for each component:
    /// bit `i` of `mask` reverses component `i`. Reversing a strand swaps
    /// incoming and outgoing at every dart along it.
    fn signs_with(&self, comp: &[usize], base: &[bool], mask: u32) -> Vec<i8> {
        let inc = |d: Dart| base[d] ^ ((mask >> comp[d]) & 1 == 1);
        (0..self.n)
            .map(|c| {
                let q = if inc(dart(c, 0)) { 0 } else { 2 };
                if inc(dart(c, q + 3)) {
                    1
                } else {
                    -1
                }
            })
            .collect()
    }

    /// Canonical code that does not depend on how the components happen to be
    /// oriented.
    ///
    /// This matters only for links. Crossing signs of a link depend on the
    /// direction chosen for each component: reversing one component negates
    /// every crossing *between* components, while self-crossings are
    /// unaffected. Nothing in a PD code fixes those directions, so a key that
    /// depended on them would change under relabelling — which is exactly what
    /// randomised testing against PD round-trips caught.
    ///
    /// So minimise over all `2^c` orientations. For a knot every crossing is a
    /// self-crossing, both choices give identical signs, and the key is
    /// unchanged.
    ///
    /// The consequence to be aware of: this key identifies *unoriented* link
    /// diagrams. That is right for unknotting and unlinking claims, and wrong
    /// for anything that depends on orientation, such as linking number, which
    /// would need to pin the orientation separately.
    fn canon_unoriented(&self) -> String {
        let (comp, ncomp) = self.dart_components();
        let base = self.orientation();
        let bits = ncomp.min(16);
        let mut best: Option<String> = None;
        for mask in 0..(1u32 << bits) {
            let code = self.canon_oriented(&self.signs_with(&comp, &base, mask));
            if best.as_ref().map_or(true, |b| code < *b) {
                best = Some(code);
            }
        }
        best.unwrap_or_default()
    }

    /// Lexicographically minimal signed Gauss code: the primary key. Correct for
    /// links as well as knots. When `mirror_invariant`, also minimised over the
    /// mirror, which is right for mirror-invariant claims such as `u`.
    pub fn canon(&self, mirror_invariant: bool) -> String {
        let here = self.canon_unoriented();
        if !mirror_invariant {
            return here;
        }
        let there = self.mirror().canon_unoriented();
        if there < here {
            there
        } else {
            here
        }
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
