//! Moves and their legality checks.
//!
//! Every move is checked *before* it is applied, and the result is re-validated
//! for planarity. A move the verifier cannot check is an error, never a
//! silently accepted step. See `schema/claim-types.md` for the alphabets.

use crate::diagram::{cr, dart, is_over, pos, sigma, sigma2, Dart, Diagram};
use std::collections::{HashMap, HashSet};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Move {
    R1Add { d: Dart, loop_pos: usize },
    R1Del { c: usize },
    R2Add { d1: Dart, d2: Dart, over: u8 },
    R2Del { c1: usize, c2: usize },
    R3 { c1: usize, c2: usize, c3: usize },
    Xc { c: usize },
}

impl Move {
    /// The smallest alphabet containing this move: 'R' or 'X'.
    pub fn alphabet(&self) -> char {
        match self {
            Move::Xc { .. } => 'X',
            _ => 'R',
        }
    }

    pub fn is_crossing_change(&self) -> bool {
        matches!(self, Move::Xc { .. })
    }

    pub fn parse(line: &str) -> Result<Move, String> {
        let line = line.split('#').next().unwrap_or("").trim();
        let mut it = line.split_whitespace();
        let op = it.next().ok_or("empty move")?;
        let mut kv: Vec<(String, String)> = Vec::new();
        for tok in it {
            let (k, v) = tok
                .split_once('=')
                .ok_or_else(|| format!("expected key=value, got `{}`", tok))?;
            kv.push((k.to_string(), v.to_string()));
        }
        let get = |k: &str| -> Result<String, String> {
            kv.iter()
                .find(|(a, _)| a == k)
                .map(|(_, b)| b.clone())
                .ok_or_else(|| format!("{}: missing `{}=`", op, k))
        };
        let num = |s: String| -> Result<usize, String> {
            s.parse::<usize>()
                .map_err(|_| format!("`{}` is not a number", s))
        };
        match op {
            "R1+" => Ok(Move::R1Add {
                d: num(get("d")?)?,
                loop_pos: num(get("loop")?)?,
            }),
            "R1-" => Ok(Move::R1Del { c: num(get("c")?)? }),
            "R2+" => Ok(Move::R2Add {
                d1: num(get("d1")?)?,
                d2: num(get("d2")?)?,
                over: num(get("over")?)? as u8,
            }),
            "R2-" => Ok(Move::R2Del {
                c1: num(get("c1")?)?,
                c2: num(get("c2")?)?,
            }),
            "R3" => Ok(Move::R3 {
                c1: num(get("c1")?)?,
                c2: num(get("c2")?)?,
                c3: num(get("c3")?)?,
            }),
            "XC" => Ok(Move::Xc { c: num(get("c")?)? }),
            other => Err(format!("unknown move `{}`", other)),
        }
    }
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Move::R1Add { d, loop_pos } => write!(f, "R1+ d={} loop={}", d, loop_pos),
            Move::R1Del { c } => write!(f, "R1- c={}", c),
            Move::R2Add { d1, d2, over } => write!(f, "R2+ d1={} d2={} over={}", d1, d2, over),
            Move::R2Del { c1, c2 } => write!(f, "R2- c1={} c2={}", c1, c2),
            Move::R3 { c1, c2, c3 } => write!(f, "R3 c1={} c2={} c3={}", c1, c2, c3),
            Move::Xc { c } => write!(f, "XC c={}", c),
        }
    }
}

impl Diagram {
    pub fn apply(&self, m: Move) -> Result<Diagram, String> {
        match m {
            Move::R1Add { d, loop_pos } => self.r1_add(d, loop_pos),
            Move::R1Del { c } => self.r1_del(c),
            Move::R2Del { c1, c2 } => self.r2_del(c1, c2),
            Move::Xc { c } => self.crossing_change(c),
            Move::R3 { c1, c2, c3 } => self.r3(c1, c2, c3),
            Move::R2Add { d1, d2, over } => self.r2_add(d1, d2, over),
        }
    }

    /// Reidemeister II, increasing: push a finger of the arc at `d1` across the
    /// arc at `d2`. The two darts must lie on a common face. `over = 1` makes
    /// the strand at `d1` pass over the other, `over = 2` under.
    ///
    /// Local picture: the shared face as a disk, arc `A` (at `d1`) along one
    /// side and arc `B` (at `d2`) along the other, each traversed in the
    /// direction the face walk takes it. The face walk keeps the face on the
    /// *right* of each directed arc — `phi = sigma . alpha` runs clockwise
    /// around an inner face — so `A` and `B` run antiparallel across the disk
    /// and meet the two new crossings in opposite orders:
    ///
    /// ```text
    ///   A:  d1 -> P -> Q -> e1
    ///   B:  d2 -> Q -> P -> e2
    /// ```
    ///
    /// Reading the four rays at each new crossing counterclockwise:
    ///
    /// ```text
    ///   at P:  [B->Q,  A->d1,  B->e2,  A->Q ]
    ///   at Q:  [B->P,  A->P,   B->d2,  A->e1]
    /// ```
    ///
    /// Opposite slots hold the same strand in both, as they must. `over` only
    /// shifts where that cyclic order starts, which is what moves `A` between
    /// the odd (over) and even (under) slots.
    ///
    /// Every one of these choices was confirmed against the exhaustive scan
    /// described in `docs/roadmap.md`: of the eight candidate wirings, exactly
    /// this one yields a planar diagram that reduces back, for every legal
    /// dart pair on every face of the test diagrams.
    pub fn r2_add(&self, d1: Dart, d2: Dart, over: u8) -> Result<Diagram, String> {
        if self.n == 0 {
            return Err("R2+ on a crossingless diagram is not supported in v0".into());
        }
        if d1 >= 4 * self.n || d2 >= 4 * self.n {
            return Err(format!(
                "R2+: dart out of range (diagram has {} darts)",
                4 * self.n
            ));
        }
        if d1 == d2 {
            return Err("R2+: needs two distinct darts".into());
        }
        if over != 1 && over != 2 {
            return Err("R2+: over must be 1 (d1 passes over) or 2 (d1 passes under)".into());
        }
        let (e1, e2) = (self.alpha[d1], self.alpha[d2]);
        if e1 == d2 {
            return Err(
                "R2+: d1 and d2 lie on the same arc; folding an arc across itself \
                 is a different construction and is not supported in v0"
                    .into(),
            );
        }
        let on_face = self
            .faces()
            .into_iter()
            .any(|f| f.contains(&d1) && f.contains(&d2));
        if !on_face {
            return Err(format!(
                "R2+: darts {} and {} do not lie on a common face",
                d1, d2
            ));
        }

        let (p, q) = (self.n, self.n + 1);
        let s = if over == 1 { 0 } else { 1 };
        let (p_b_q, p_a_d1) = (dart(p, s), dart(p, 1 + s));
        let (p_b_e2, p_a_q) = (dart(p, 2 + s), dart(p, 3 + s));
        let (q_b_p, q_a_p) = (dart(q, s), dart(q, 1 + s));
        let (q_b_d2, q_a_e1) = (dart(q, 2 + s), dart(q, 3 + s));

        let mut alpha = self.alpha.clone();
        alpha.extend_from_slice(&[usize::MAX; 8]);
        for (x, y) in [
            (d1, p_a_d1),   // A: from the d1 end into P
            (p_a_q, q_a_p), // A: the finger between the two new crossings
            (q_a_e1, e1),   // A: out of Q to the far end
            (d2, q_b_d2),   // B: from the d2 end into Q
            (q_b_p, p_b_q), // B: the bigon's other side
            (p_b_e2, e2),   // B: out of P to the far end
        ] {
            alpha[x] = y;
            alpha[y] = x;
        }

        let out = Diagram {
            n: self.n + 2,
            alpha,
            free_loops: self.free_loops,
        };
        out.validate()
            .map_err(|e| format!("R2+ produced an invalid diagram: {}", e))?;
        Ok(out)
    }

    /// Swap over and under at one crossing by rotating its darts one step.
    /// Rotation by 1 exchanges positions {0,2} with {1,3}, i.e. under with over.
    pub fn crossing_change(&self, c: usize) -> Result<Diagram, String> {
        if c >= self.n {
            return Err(format!("XC: no crossing {} (diagram has {})", c, self.n));
        }
        let rho = |x: Dart| if cr(x) == c { dart(c, pos(x) + 3) } else { x };
        let rho_inv = |x: Dart| if cr(x) == c { dart(c, pos(x) + 1) } else { x };
        let alpha: Vec<Dart> = (0..4 * self.n)
            .map(|x| rho(self.alpha[rho_inv(x)]))
            .collect();
        let out = Diagram {
            n: self.n,
            alpha,
            free_loops: self.free_loops,
        };
        out.validate()?;
        Ok(out)
    }

    /// Delete crossings and reconnect the strands that ran through them.
    /// Shared by R1- and R2-; strands are followed through the deleted region
    /// via the through-map `sigma^2`.
    pub fn splice(&self, deleted: &[usize]) -> Result<Diagram, String> {
        let del: HashSet<usize> = deleted.iter().cloned().collect();
        for &c in &del {
            if c >= self.n {
                return Err(format!("splice: no crossing {}", c));
            }
        }
        let survivors: Vec<usize> = (0..self.n).filter(|c| !del.contains(c)).collect();
        let mut newidx = vec![usize::MAX; self.n];
        for (i, &c) in survivors.iter().enumerate() {
            newidx[c] = i;
        }
        let m = survivors.len();
        let mut alpha = vec![usize::MAX; 4 * m];
        let mut touched = vec![false; 4 * self.n];

        for &c in &survivors {
            for p in 0..4 {
                let s = dart(c, p);
                let mut t = self.alpha[s];
                let mut guard = 0;
                while del.contains(&cr(t)) {
                    touched[t] = true;
                    let o = sigma2(t);
                    touched[o] = true;
                    t = self.alpha[o];
                    guard += 1;
                    if guard > 4 * self.n + 4 {
                        return Err("splice: strand never leaves the deleted region".into());
                    }
                }
                alpha[dart(newidx[c], p)] = dart(newidx[cr(t)], pos(t));
            }
        }

        // Whatever is left inside the deleted region is a closed circle. Each
        // circle contributes two orbits (its incoming and outgoing darts).
        let mut seen = touched;
        let mut orbits = 0;
        for d0 in 0..4 * self.n {
            if seen[d0] || !del.contains(&cr(d0)) {
                continue;
            }
            orbits += 1;
            let mut d = d0;
            loop {
                seen[d] = true;
                d = self.alpha[sigma2(d)];
                if d == d0 {
                    break;
                }
            }
        }

        let out = Diagram {
            n: m,
            alpha,
            free_loops: self.free_loops + orbits / 2,
        };
        out.validate()?;
        Ok(out)
    }

    /// A kink is a monogon face: `phi(d) = d` for some dart, equivalently
    /// `alpha[4c+q] == 4c+(q+3)`.
    pub fn is_kink(&self, c: usize) -> bool {
        c < self.n && (0..4).any(|q| self.alpha[dart(c, q)] == dart(c, q + 3))
    }

    pub fn r1_del(&self, c: usize) -> Result<Diagram, String> {
        if c >= self.n {
            return Err(format!("R1-: no crossing {} (diagram has {})", c, self.n));
        }
        if !self.is_kink(c) {
            return Err(format!("R1-: crossing {} is not a kink", c));
        }
        self.splice(&[c])
    }

    /// Insert a kink on the arc at dart `d`. `loop_pos` in 0..3 selects which
    /// adjacent pair of the new crossing forms the loop, which fixes both the
    /// side of the kink and whether the strand passes over or under itself.
    pub fn r1_add(&self, d: Dart, loop_pos: usize) -> Result<Diagram, String> {
        if self.n == 0 {
            return Err("R1+ on a crossingless diagram is not supported in v0".into());
        }
        if d >= 4 * self.n {
            return Err(format!("R1+: no dart {} (diagram has {})", d, 4 * self.n));
        }
        let c = self.n;
        let e = self.alpha[d];
        let mut alpha = self.alpha.clone();
        alpha.extend_from_slice(&[usize::MAX; 4]);
        let lp = loop_pos & 3;
        let (l0, l1) = (dart(c, lp), dart(c, lp + 1));
        let (f1, f2) = (dart(c, lp + 2), dart(c, lp + 3));
        alpha[l0] = l1;
        alpha[l1] = l0;
        alpha[f2] = d;
        alpha[d] = f2;
        alpha[f1] = e;
        alpha[e] = f1;
        let out = Diagram {
            n: c + 1,
            alpha,
            free_loops: self.free_loops,
        };
        out.validate()?;
        Ok(out)
    }

    /// Find a bigon face on crossings `c1` and `c2` that is a genuine R2 pair:
    /// one strand runs over at both crossings and the other under at both. A
    /// clasp also bounds a bigon but interleaves, and is rejected.
    pub fn r2_bigon(&self, c1: usize, c2: usize) -> Option<Vec<Dart>> {
        if c1 == c2 || c1 >= self.n || c2 >= self.n {
            return None;
        }
        self.faces().into_iter().find(|f| {
            f.len() == 2
                && {
                    let (a, b) = (cr(f[0]), cr(f[1]));
                    (a == c1 && b == c2) || (a == c2 && b == c1)
                }
                && is_over(f[0]) == is_over(self.alpha[f[0]])
        })
    }

    pub fn r2_del(&self, c1: usize, c2: usize) -> Result<Diagram, String> {
        if c1 >= self.n || c2 >= self.n {
            return Err(format!("R2-: no such crossings {} {}", c1, c2));
        }
        if c1 == c2 {
            return Err("R2-: needs two distinct crossings".into());
        }
        if self.r2_bigon(c1, c2).is_none() {
            return Err(format!(
                "R2-: crossings {} and {} do not bound a removable bigon",
                c1, c2
            ));
        }
        self.splice(&[c1, c2])
    }

    /// Data of a triangular face, in phi-order: the triangle darts `d[i]`,
    /// their arc partners `a[i] = alpha(d[i])`, and the two outer darts at each
    /// crossing, `u[i] = sigma(d[i])` and `v[i] = sigma(u[i])`.
    ///
    /// Strand `E_i` runs from outer dart `v[i]` at crossing `cr(d[i])`, through
    /// the arc `{d[i], a[i]}`, to outer dart `u[i+1]` at `cr(d[i+1])`.
    fn triangle_data(&self, f: &[Dart]) -> Option<([Dart; 3], [Dart; 3], [Dart; 3], [Dart; 3])> {
        if f.len() != 3 {
            return None;
        }
        let d = [f[0], f[1], f[2]];
        if cr(d[0]) == cr(d[1]) || cr(d[1]) == cr(d[2]) || cr(d[0]) == cr(d[2]) {
            return None;
        }
        let a = [self.alpha[d[0]], self.alpha[d[1]], self.alpha[d[2]]];
        let u = [sigma(d[0]), sigma(d[1]), sigma(d[2])];
        let v = [sigma(u[0]), sigma(u[1]), sigma(u[2])];
        Some((d, a, u, v))
    }

    /// A triangle admits R3 only if it is *layered*: of the three strands, one
    /// runs over at both its crossings, one under at both, one over at one and
    /// under at the other. A cyclic over/under pattern is not an R3 triangle.
    fn is_layered(d: &[Dart; 3], a: &[Dart; 3]) -> bool {
        let (mut tt, mut ff, mut mixed) = (0, 0, 0);
        for i in 0..3 {
            match (is_over(d[i]), is_over(a[i])) {
                (true, true) => tt += 1,
                (false, false) => ff += 1,
                _ => mixed += 1,
            }
        }
        tt == 1 && ff == 1 && mixed == 1
    }

    /// Every triangular face that admits an R3 move.
    pub fn triangles(&self) -> Vec<(usize, usize, usize)> {
        self.faces()
            .iter()
            .filter_map(|f| {
                let (d, a, _, _) = self.triangle_data(f)?;
                if Diagram::is_layered(&d, &a) {
                    Some((cr(d[0]), cr(d[1]), cr(d[2])))
                } else {
                    None
                }
            })
            .collect()
    }

    /// The Reidemeister III move on the triangular face spanned by the three
    /// given crossings.
    ///
    /// The move slides a strand across the opposite crossing, which flips the
    /// triangle to the other side. At every crossing the triangle corner moves
    /// to the *opposite* corner, and over/under is preserved, which pins the
    /// strand `E_{i-1}` to the slot pair `{p, p+2}` it already occupies. That
    /// determines the new wiring uniquely, with no choice left to make:
    ///
    /// - the outer arc that met slot `v[i]` now meets slot `a[i]`
    /// - the outer arc that met slot `u[i+1]` now meets slot `d[i]`
    /// - the three internal arcs become `{u[i+1], v[i]}`
    ///
    /// Slots keep their positions, so the invariant "under at 0 and 2" is
    /// untouched. Planarity is re-checked on the result regardless.
    pub fn r3(&self, c1: usize, c2: usize, c3: usize) -> Result<Diagram, String> {
        let want: HashSet<usize> = [c1, c2, c3].into_iter().collect();
        if want.len() != 3 {
            return Err("R3: needs three distinct crossings".into());
        }
        for c in [c1, c2, c3] {
            if c >= self.n {
                return Err(format!("R3: no crossing {} (diagram has {})", c, self.n));
            }
        }
        let face = self
            .faces()
            .into_iter()
            .find(|f| f.len() == 3 && f.iter().map(|&x| cr(x)).collect::<HashSet<_>>() == want)
            .ok_or_else(|| {
                format!(
                    "R3: crossings {}, {}, {} do not bound a triangular face",
                    c1, c2, c3
                )
            })?;

        let (d, a, u, v) = self
            .triangle_data(&face)
            .ok_or("R3: degenerate triangular face")?;
        if !Diagram::is_layered(&d, &a) {
            return Err(format!(
                "R3: the triangle on {}, {}, {} is not layered; no strand runs \
                 over both others",
                c1, c2, c3
            ));
        }

        // Where each outer slot's arc moves to.
        let mut mu: HashMap<Dart, Dart> = HashMap::new();
        for i in 0..3 {
            mu.insert(v[i], a[i]);
            mu.insert(u[(i + 1) % 3], d[i]);
        }
        let outer: HashSet<Dart> = mu.keys().cloned().collect();

        let mut alpha = self.alpha.clone();
        // Re-attach the six outer arcs. An arc with both ends outer (possible in
        // small diagrams) is moved at both ends at once.
        let mut done: HashSet<Dart> = HashSet::new();
        for (&x, &nx) in &mu {
            if done.contains(&x) {
                continue;
            }
            let y = self.alpha[x];
            let ny = if outer.contains(&y) {
                done.insert(y);
                mu[&y]
            } else {
                y
            };
            done.insert(x);
            alpha[nx] = ny;
            alpha[ny] = nx;
        }
        // The three internal arcs of the flipped triangle.
        for i in 0..3 {
            let (p, q) = (u[(i + 1) % 3], v[i]);
            alpha[p] = q;
            alpha[q] = p;
        }

        let out = Diagram {
            n: self.n,
            alpha,
            free_loops: self.free_loops,
        };
        out.validate()
            .map_err(|e| format!("R3 produced an invalid diagram: {}", e))?;
        Ok(out)
    }

    /// Greedy R1-/R2- reduction. Returns the reduced diagram and the trace.
    /// This is the bootstrap *producer*: humans do not author traces by hand.
    pub fn reduce(&self) -> (Diagram, Vec<Move>) {
        let mut d = self.clone();
        let mut trace = Vec::new();
        loop {
            let faces = d.faces();
            if let Some(f) = faces.iter().find(|f| f.len() == 1) {
                let c = cr(f[0]);
                let m = Move::R1Del { c };
                match d.apply(m) {
                    Ok(next) => {
                        trace.push(m);
                        d = next;
                        continue;
                    }
                    Err(_) => {}
                }
            }
            let bigon = faces.iter().find(|f| {
                f.len() == 2 && cr(f[0]) != cr(f[1]) && is_over(f[0]) == is_over(d.alpha[f[0]])
            });
            if let Some(f) = bigon {
                let (c1, c2) = (cr(f[0]), cr(f[1]));
                let m = Move::R2Del { c1, c2 };
                if let Ok(next) = d.apply(m) {
                    trace.push(m);
                    d = next;
                    continue;
                }
            }
            return (d, trace);
        }
    }
}
