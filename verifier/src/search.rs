//! Bootstrap producer: find an unknotting trace so certificates can be
//! generated rather than hand-authored.
//!
//! Greedy R1-/R2- reduction, a bounded breadth-first search over R3 moves to
//! unlock further reduction, and crossing changes on top. There is still no
//! R2+, so diagrams that must first be made *more* complicated are out of
//! reach — including, by construction, hard unknot diagrams. When the search
//! fails it says so; it never guesses.

use crate::diagram::Diagram;
use crate::moves::Move;
use std::collections::HashSet;

/// Reduce as far as possible, searching up to `r3_depth` R3 moves deep for a
/// configuration that unlocks an R1 or R2. R3 preserves the crossing count, so
/// it is only worth making when something reducible follows.
pub fn simplify(d: &Diagram, r3_depth: usize, cap: usize) -> (Diagram, Vec<Move>) {
    let (base, base_trace) = d.reduce();
    if base.n == 0 || r3_depth == 0 {
        return (base, base_trace);
    }
    let target = base.n;
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(base.canon(false));
    let mut frontier = vec![(base.clone(), base_trace.clone())];

    for _ in 0..r3_depth {
        let mut next = Vec::new();
        for (cur, tr) in frontier.drain(..) {
            for (c1, c2, c3) in cur.triangles() {
                let m = Move::R3 { c1, c2, c3 };
                let after = match cur.apply(m) {
                    Ok(x) => x,
                    Err(_) => continue,
                };
                let (red, rtr) = after.reduce();
                let mut t2 = tr.clone();
                t2.push(m);
                t2.extend(rtr);
                if red.n < target {
                    // The crossing count strictly dropped, so recursing here
                    // terminates; let the win compound.
                    let (fin, ftr) = simplify(&red, r3_depth, cap);
                    t2.extend(ftr);
                    return (fin, t2);
                }
                if seen.len() >= cap {
                    return (base, base_trace);
                }
                if seen.insert(red.canon(false)) {
                    next.push((red, t2));
                }
            }
        }
        frontier = next;
        if frontier.is_empty() {
            break;
        }
    }
    (base, base_trace)
}

pub struct Search {
    seen: HashSet<String>,
    nodes: usize,
    cap: usize,
    r3_depth: usize,
}

impl Search {
    pub fn new(cap: usize, r3_depth: usize) -> Search {
        Search {
            seen: HashSet::new(),
            nodes: 0,
            cap,
            r3_depth,
        }
    }

    fn go(&mut self, d: &Diagram, budget: usize) -> Option<Vec<Move>> {
        let (red, mut trace) = simplify(d, self.r3_depth, self.cap);
        if red.n == 0 {
            return Some(trace);
        }
        if budget == 0 {
            return None;
        }
        if self.nodes >= self.cap {
            return None;
        }
        self.nodes += 1;
        let key = format!("{}|{}", budget, red.canon(false));
        if !self.seen.insert(key) {
            return None;
        }
        for c in 0..red.n {
            let next = match red.crossing_change(c) {
                Ok(x) => x,
                Err(_) => continue,
            };
            if let Some(rest) = self.go(&next, budget - 1) {
                trace.push(Move::Xc { c });
                trace.extend(rest);
                return Some(trace);
            }
        }
        None
    }
}

/// Iterative deepening, so the first trace found uses the fewest crossing
/// changes this move set can achieve. That is an upper bound on `u`, never a
/// proof of minimality.
pub fn unknotting_trace(
    d: &Diagram,
    max_u: usize,
    cap: usize,
    r3_depth: usize,
) -> Option<(usize, Vec<Move>)> {
    for budget in 0..=max_u {
        let mut s = Search::new(cap, r3_depth);
        if let Some(trace) = s.go(d, budget) {
            let cost = trace.iter().filter(|m| m.is_crossing_change()).count();
            return Some((cost, trace));
        }
    }
    None
}
