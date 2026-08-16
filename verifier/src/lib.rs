//! Machine-checked certificates for knot theory.
//!
//! The verifier is deliberately dependency-free: it must be readable end to end
//! and must not inherit trust from Regina, SnapPy, or a YAML library. See
//! `schema/` for the normative formats.
//!
//! ```
//! use unknotdb::diagram::Diagram;
//! let trefoil = Diagram::from_braid(2, &[1, 1, 1]).unwrap();
//! assert_eq!(trefoil.writhe(), 3);
//! ```

pub mod cert;
pub mod diagram;
pub mod moves;
pub mod search;
pub mod util;

#[cfg(test)]
mod tests {
    use crate::diagram::Diagram;
    use crate::search::unknotting_trace;

    fn trefoil() -> Diagram {
        Diagram::from_braid(2, &[1, 1, 1]).unwrap()
    }

    #[test]
    fn trefoil_structure() {
        let t = trefoil();
        assert_eq!(t.n, 3);
        assert_eq!(t.faces().len(), 5, "V - E + F = 2");
        assert_eq!(t.components(), 1);
        assert_eq!(t.writhe().abs(), 3);
    }

    #[test]
    fn pd_roundtrip() {
        let t = trefoil();
        let back = Diagram::from_pd(&t.to_pd()).unwrap();
        assert_eq!(back.canon(false), t.canon(false));
    }

    #[test]
    fn canon_is_start_independent() {
        let t = trefoil();
        // Re-reading our own PD relabels every arc; the key must not move.
        let relabelled = Diagram::from_pd(&t.to_pd()).unwrap();
        assert_eq!(t.canon(true), relabelled.canon(true));
    }

    #[test]
    fn r1_add_then_reduce_is_identity() {
        let t = trefoil();
        for loop_pos in 0..4 {
            let bigger = t.r1_add(0, loop_pos).expect("R1+ is always legal");
            assert_eq!(bigger.n, 4);
            let (red, trace) = bigger.reduce();
            assert!(!trace.is_empty());
            assert_eq!(red.n, 3);
            assert_eq!(red.canon(false), t.canon(false));
        }
    }

    #[test]
    fn cancelling_pair_closes_to_the_two_component_unlink() {
        // The closure of the trivial 2-braid is two circles, not the unknot.
        let d = Diagram::from_braid(2, &[1, -1]).unwrap();
        let (red, trace) = d.reduce();
        assert_eq!(trace.len(), 1);
        assert_eq!(red.n, 0);
        assert_eq!(red.free_loops, 2);
        assert_eq!(red.components(), 2);
    }

    #[test]
    fn single_generator_closes_to_the_unknot() {
        let d = Diagram::from_braid(2, &[1]).unwrap();
        assert_eq!(d.components(), 1);
        assert!(d.is_kink(0));
        let (red, _) = d.reduce();
        assert_eq!(red.n, 0);
        assert_eq!(red.free_loops, 1);
    }

    /// B_1 has no generators; its closure is one circle, the unknot. Found by
    /// running the parser over rf-knots' evidence corpus, which seeds unknot
    /// rungs as one-strand braids.
    #[test]
    fn one_strand_braid_is_the_unknot() {
        let d = Diagram::from_braid(1, &[]).expect("B_1 is legal");
        assert_eq!(d.n, 0);
        assert_eq!(d.free_loops, 1);
        assert_eq!(d.components(), 1);
        assert_eq!(d.canon(true), "U");
        // ...but B_1 has no generators, so any letter is still out of range.
        assert!(Diagram::from_braid(1, &[1]).is_err());
        assert!(Diagram::from_braid(0, &[]).is_err());
    }

    /// The closure of sigma_1 sigma_3 in B_4 is two disjoint kinks: a split
    /// diagram. Euler's formula holds per connected component, so it has
    /// n + 2k = 6 faces, not n + 2. Found by random differential testing
    /// against rf-knots.
    #[test]
    fn split_diagrams_are_valid() {
        let d = Diagram::from_braid(4, &[1, 3]).expect("a split closure is legal");
        assert_eq!(d.n, 2);
        assert_eq!(d.map_components(), 2, "two disjoint kinks");
        assert_eq!(d.faces().len(), d.n + 2 * d.map_components());
        assert_eq!(d.components(), 2, "two link components");
        let (red, _) = d.reduce();
        assert_eq!(red.n, 0, "both kinks come out");
        assert_eq!(red.free_loops, 2, "leaving a 2-component unlink");
    }

    /// Cyclic (cylinder) braids: the seam generator is the BKL band
    /// a_{1,n} = w sigma_1 w^-1, an ordinary element of B_n, so the closure is
    /// an ordinary link. Checked letter-for-letter against rf-knots'
    /// compile_cyclic_bands over 480 random words elsewhere; this pins the
    /// expansion itself.
    #[test]
    fn cyclic_seam_generator_expands_to_a_band() {
        assert_eq!(Diagram::compile_cyclic(3, &[3]).unwrap(), vec![2, 1, -2]);
        assert_eq!(Diagram::compile_cyclic(3, &[-3]).unwrap(), vec![2, -1, -2]);
        assert_eq!(
            Diagram::compile_cyclic(4, &[4]).unwrap(),
            vec![3, 2, 1, -2, -3]
        );
        assert_eq!(Diagram::compile_cyclic(3, &[1, 2]).unwrap(), vec![1, 2]);
        assert_eq!(Diagram::compile_cyclic(2, &[2]).unwrap(), vec![1]);
        // exactly one letter of the expansion is the band's own crossing, so a
        // seam flip stays a single crossing change
        let pos = Diagram::compile_cyclic(5, &[5]).unwrap();
        let neg = Diagram::compile_cyclic(5, &[-5]).unwrap();
        assert_eq!(pos.len(), neg.len());
        assert_eq!(pos.iter().zip(&neg).filter(|(a, b)| a != b).count(), 1);
        assert!(Diagram::compile_cyclic(3, &[4]).is_err());
        assert!(Diagram::from_cyclic_braid(3, &[1, 3, 2]).is_ok());
    }

    /// `canon` used to walk 2n steps from one dart, which on a link cycles
    /// inside a single component and repeats instead of covering the diagram.
    /// It is the primary key, so silent nonsense there is the worst kind of bug.
    #[test]
    fn canon_is_correct_for_links() {
        let hopf = Diagram::from_braid(2, &[1, 1]).unwrap();
        let code = hopf.canon(false);
        assert_eq!(code.matches('|').count(), 1, "two components: {}", code);
        // each of the two crossings is met once by each component
        assert_eq!(code.matches('1').count(), 2, "{}", code);
        assert_eq!(code.matches('2').count(), 2, "{}", code);

        // a split diagram: two independent kinks, no shared crossings
        let split = Diagram::from_braid(4, &[1, 3]).unwrap();
        assert_eq!(split.canon(false), "O1+U1+|O2+U2+");

        // unlinks are distinguished by component count
        assert_eq!(Diagram::from_braid(2, &[]).unwrap().canon(false), "U|U");
        assert_eq!(Diagram::from_braid(3, &[]).unwrap().canon(false), "U|U|U");
    }

    /// A knot and that knot split off from a free circle are different links.
    /// Free circles are tracked outside the dart structure, so they have to be
    /// appended to the key explicitly or the two collide.
    #[test]
    fn a_free_circle_changes_the_key() {
        let trefoil = Diagram::from_braid(2, &[1, 1, 1]).unwrap();
        let with_circle = Diagram::from_braid(3, &[1, 1, 1]).unwrap();
        assert_eq!(with_circle.free_loops, 1);
        assert_eq!(with_circle.components(), 2);
        assert_ne!(trefoil.canon(true), with_circle.canon(true));
        assert_eq!(
            with_circle.canon(true),
            format!("{}|U", trefoil.canon(true))
        );
    }

    /// Crossing signs of a link depend on the direction chosen for each
    /// component: reversing one negates every crossing between components.
    /// Nothing in a PD code fixes those directions, so a key that depended on
    /// them changes under relabelling. These words are ones that actually broke
    /// before the key was minimised over component orientations.
    #[test]
    fn link_keys_do_not_depend_on_component_orientation() {
        for (s, w) in [
            (4, vec![-2, 3, -1, -3]),
            (4, vec![2, 2, 1, -1, 3, -1, 3]),
            (4, vec![2, -2, -1, 1, 3, -2, -2]),
        ] {
            let d = Diagram::from_braid(s, &w).unwrap();
            assert!(d.canon(false).contains('|'), "expected a link: {:?}", w);
            let relabelled = Diagram::from_pd(&d.to_pd()).unwrap();
            assert_eq!(d.canon(false), relabelled.canon(false), "braid {:?}", w);
            assert_eq!(d.canon(true), relabelled.canon(true), "braid {:?}", w);
        }
    }

    /// The key must not depend on how the diagram happens to be labelled.
    #[test]
    fn link_keys_survive_relabelling() {
        for (s, w) in [(2, vec![1, 1]), (4, vec![1, 3]), (3, vec![1, 1, 2, -1])] {
            let d = Diagram::from_braid(s, &w).unwrap();
            let round_tripped = Diagram::from_pd(&d.to_pd()).unwrap();
            assert_eq!(d.canon(true), round_tripped.canon(true), "braid {:?}", w);
        }
    }

    #[test]
    fn hopf_link_has_two_components() {
        let d = Diagram::from_braid(2, &[1, 1]).unwrap();
        assert_eq!(d.components(), 2);
    }

    #[test]
    fn crossing_change_is_an_involution() {
        let t = trefoil();
        let once = t.crossing_change(0).unwrap();
        let twice = once.crossing_change(0).unwrap();
        assert_eq!(twice.canon(false), t.canon(false));
        assert_ne!(once.canon(false), t.canon(false));
    }

    #[test]
    fn mirror_negates_writhe() {
        let t = trefoil();
        assert_eq!(t.writhe(), -t.mirror().writhe());
    }

    #[test]
    fn trefoil_has_unknotting_number_at_most_one() {
        let t = trefoil();
        let (cost, trace) = unknotting_trace(&t, 3, 100_000, 2).expect("trace exists");
        assert_eq!(cost, 1);
        let mut d = t.clone();
        for m in &trace {
            d = d.apply(*m).expect("replay");
        }
        assert_eq!(d.n, 0);
    }

    #[test]
    fn cinquefoil_needs_two_crossing_changes() {
        let k = Diagram::from_braid(2, &[1, 1, 1, 1, 1]).unwrap();
        let (cost, _) = unknotting_trace(&k, 3, 500_000, 2).expect("trace exists");
        assert_eq!(cost, 2);
    }

    /// Ground truth for R3: the braid relation *is* an R3 move. Applying the
    /// relation inside a longer word gives two diagrams that differ as
    /// diagrams but represent the same link, so this discriminates a correct
    /// rewiring from a merely plausible one.
    #[test]
    fn r3_realises_the_braid_relation() {
        let cases: [(usize, Vec<i32>, Vec<i32>); 5] = [
            (3, vec![1, 2, 1, 1], vec![2, 1, 2, 1]),
            (3, vec![1, 2, 1, 2], vec![2, 1, 2, 2]),
            (3, vec![1, 2, 1, -2], vec![2, 1, 2, -2]),
            (4, vec![1, 2, 1, 3], vec![2, 1, 2, 3]),
            (4, vec![1, 2, 1, 3, 2], vec![2, 1, 2, 3, 2]),
        ];
        for (s, wa, wb) in cases {
            let a = Diagram::from_braid(s, &wa).unwrap();
            let b = Diagram::from_braid(s, &wb).unwrap();
            assert_ne!(
                a.canon(false),
                b.canon(false),
                "{:?} is not a real test",
                wa
            );
            let tris = a.triangles();
            assert!(!tris.is_empty(), "no layered triangle in {:?}", wa);
            let hit = tris.iter().any(|&(c1, c2, c3)| {
                a.r3(c1, c2, c3)
                    .map(|x| x.canon(false) == b.canon(false))
                    .unwrap_or(false)
            });
            assert!(hit, "R3 did not realise the braid relation for {:?}", wa);
        }
    }

    #[test]
    fn r3_is_an_involution_and_preserves_writhe() {
        let a = Diagram::from_braid(3, &[1, 2, 1, 1]).unwrap();
        let tris = a.triangles();
        assert!(!tris.is_empty());
        for (c1, c2, c3) in tris {
            let once = a.r3(c1, c2, c3).expect("R3 legal");
            assert_eq!(once.n, a.n, "R3 must not change the crossing count");
            assert_eq!(once.writhe(), a.writhe(), "R3 must not change the writhe");
            assert_eq!(once.components(), a.components());
            assert_ne!(once.canon(false), a.canon(false), "R3 must do something");
            let twice = once.r3(c1, c2, c3).expect("the triangle persists");
            assert_eq!(twice.canon(false), a.canon(false), "R3 is an involution");
        }
    }

    /// 8_19 = T(3,4) cannot be unknotted by R1/R2 reduction alone; it was the
    /// diagram that failed before R3 existed. Kronheimer-Mrowka give
    /// u(T(p,q)) = (p-1)(q-1)/2, so the answer must be 3.
    #[test]
    fn r3_unlocks_the_torus_knot_8_19() {
        let k = Diagram::from_braid(3, &[1, 2, 1, 2, 1, 2, 1, 2]).unwrap();
        assert!(
            unknotting_trace(&k, 4, 200_000, 0).is_none(),
            "without R3 this must fail"
        );
        let (cost, trace) = unknotting_trace(&k, 4, 200_000, 2).expect("with R3 it succeeds");
        assert_eq!(cost, 3);
        let mut d = k.clone();
        for m in &trace {
            d = d.apply(*m).expect("replay");
        }
        assert_eq!(d.n, 0);
        assert_eq!(d.free_loops, 1);
    }

    /// R2+ must be undone by plain reduction, and must not disturb writhe
    /// (it adds one positive and one negative crossing) or the component
    /// count. Exercised over every legal dart pair on every face.
    #[test]
    fn r2_add_is_undone_by_reduction() {
        let cases = [
            Diagram::from_braid(2, &[1, 1, 1]).unwrap(),
            Diagram::from_braid(3, &[1, -2, 1, -2]).unwrap(),
            Diagram::from_braid(3, &[1, 2, 1, 1]).unwrap(),
            Diagram::from_braid(3, &[1, 2, 1, 2, 1, 2, 1, 2]).unwrap(),
        ];
        let mut tried = 0;
        for k in cases {
            // The input need not be reduced, so compare like with like.
            let (base, _) = k.reduce();
            for face in k.faces() {
                for &d1 in &face {
                    for &d2 in &face {
                        if d1 == d2 || k.alpha[d1] == d2 {
                            continue;
                        }
                        for over in [1u8, 2] {
                            let big = k.r2_add(d1, d2, over).unwrap_or_else(|e| {
                                panic!("R2+ d1={} d2={} over={}: {}", d1, d2, over, e)
                            });
                            tried += 1;
                            assert_eq!(big.n, k.n + 2);
                            assert_eq!(big.writhe(), k.writhe(), "R2+ must preserve writhe");
                            assert_eq!(big.components(), k.components());
                            let (red, _) = big.reduce();
                            assert_eq!(
                                red.canon(false),
                                base.canon(false),
                                "R2+ d1={} d2={} over={} did not reduce back",
                                d1,
                                d2,
                                over
                            );
                        }
                    }
                }
            }
        }
        assert!(tried > 150, "only {} dart pairs exercised", tried);
    }

    /// `over` must actually control which strand passes over. Note the two
    /// results can still be isomorphic diagrams when the face is symmetric
    /// enough — on the trefoil they are — so the semantics, not the canonical
    /// form, is what to assert.
    #[test]
    fn r2_add_over_parameter_controls_the_crossing() {
        use crate::diagram::is_over;
        let mut ever_differed = false;
        for k in [
            Diagram::from_braid(2, &[1, 1, 1]).unwrap(),
            Diagram::from_braid(3, &[1, -2, 1, -2]).unwrap(),
            Diagram::from_braid(3, &[1, 2, 1, 2, 1, 2, 1, 2]).unwrap(),
        ] {
            for face in k.faces() {
                for &d1 in &face {
                    for &d2 in &face {
                        if d1 == d2 || k.alpha[d1] == d2 {
                            continue;
                        }
                        let a = k.r2_add(d1, d2, 1).unwrap();
                        let b = k.r2_add(d1, d2, 2).unwrap();
                        // alpha[d1] is the new crossing's dart on the strand
                        // that came from d1.
                        assert!(is_over(a.alpha[d1]), "over=1 must put d1's strand over");
                        assert!(!is_over(b.alpha[d1]), "over=2 must put d1's strand under");
                        ever_differed |= a.canon(false) != b.canon(false);
                    }
                }
            }
        }
        assert!(
            ever_differed,
            "over is a no-op on every input; that cannot be right"
        );
    }

    #[test]
    fn r2_add_needs_a_common_face_and_distinct_arcs() {
        let k = Diagram::from_braid(3, &[1, 2, 1, 1]).unwrap();
        assert!(k.r2_add(0, k.alpha[0], 1).is_err(), "same arc");
        assert!(k.r2_add(0, 0, 1).is_err(), "same dart");
        assert!(k.r2_add(0, 1, 7).is_err(), "over must be 1 or 2");
        let stranger =
            (0..4 * k.n).find(|&x| !k.faces().iter().any(|f| f.contains(&0) && f.contains(&x)));
        if let Some(x) = stranger {
            assert!(k.r2_add(0, x, 1).is_err(), "no common face");
        }
    }

    /// A certificate whose trace uses R2+ must verify: the verifier accepts
    /// crossing-increasing traces even though the bootstrap search never
    /// produces them.
    #[test]
    fn a_certificate_using_r2_plus_verifies() {
        let k = Diagram::from_braid(2, &[1, 1, 1]).unwrap();
        let face = k.faces().into_iter().find(|f| f.len() >= 2).unwrap();
        let (d1, d2) = (face[0], face[1]);
        let big = k.r2_add(d1, d2, 1).unwrap();
        let (cost, tail) = unknotting_trace(&big, 3, 100_000, 2).expect("trace");
        assert_eq!(cost, 1);
        let mut trace = vec![crate::moves::Move::R2Add { d1, d2, over: 1 }];
        trace.extend(tail);
        let text = crate::cert::emit(
            "unknotting_number_le",
            1,
            &k,
            &trace,
            Some("3_1"),
            "test",
            "test",
            "2026-08-15",
        );
        let report = crate::cert::Cert::parse(&text)
            .and_then(|c| c.verify())
            .expect("certificate with R2+ verifies");
        assert_eq!(report.cost, 1);
        assert!(trace
            .iter()
            .any(|m| matches!(m, crate::moves::Move::R2Add { .. })));
    }

    #[test]
    fn r3_on_a_non_triangle_is_rejected() {
        let t = Diagram::from_braid(2, &[1, 1, 1]).unwrap();
        assert!(t.r3(0, 1, 2).is_err());
        assert!(t.r3(0, 0, 1).is_err());
    }

    #[test]
    fn unpaired_arc_label_is_rejected() {
        assert!(Diagram::from_pd("PD[X[1,2,3,4]]").is_err());
    }

    #[test]
    fn non_kink_r1_removal_is_rejected() {
        let t = trefoil();
        assert!(t.r1_del(0).is_err());
    }

    #[test]
    fn clasp_bigon_is_not_removable() {
        // sigma_1^2 bounds a bigon but the strands interleave: not an R2 pair.
        let hopf = Diagram::from_braid(2, &[1, 1]).unwrap();
        assert!(hopf.r2_del(0, 1).is_err());
    }

    #[test]
    fn cancelling_bigon_is_removable() {
        let d = Diagram::from_braid(2, &[1, -1]).unwrap();
        assert!(d.r2_del(0, 1).is_ok());
    }
}
