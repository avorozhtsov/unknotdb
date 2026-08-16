mod cert;
mod diagram;
mod moves;
mod search;
mod util;

use cert::Cert;
use diagram::Diagram;
use search::unknotting_trace;

const USAGE: &str = "\
unknotdb - machine-checked certificates for knot theory

usage:
  unknotdb verify <file>...          verify certificates
  unknotdb canon  <input>            canonical signed Gauss signature (the key)
  unknotdb pd     <input>            normalised PD code
  unknotdb info   <input>            crossings, faces, components, writhe
  unknotdb reduce <input>            greedy R1-/R2- reduction, prints the trace
  unknotdb unknot <input> [--max-u N] [--cap N] [--r3 N]
                                     search for an unknotting trace
  unknotdb tri    <input>            triangular faces that admit an R3 move
  unknotdb mkcert <input> [--max-u N] [--r3 N] [--knotinfo NAME] [--source S]
                          [--date D]
                                     emit a certificate for the trace found
  unknotdb id     <file>             content-addressed certificate filename

input is either a PD code           PD[X[1,4,2,5], X[3,6,4,1], X[5,2,6,3]]
or a braid closure                  braid:2:1,1,1
";

fn parse_input(s: &str) -> Result<Diagram, String> {
    if let Some(rest) = s.strip_prefix("braid:") {
        let mut it = rest.splitn(2, ':');
        let strands: usize = it
            .next()
            .ok_or("braid: missing strand count")?
            .trim()
            .parse()
            .map_err(|_| "braid: strand count is not a number".to_string())?;
        let word: Result<Vec<i32>, _> = it
            .next()
            .unwrap_or("")
            .split(',')
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.trim().parse::<i32>())
            .collect();
        Diagram::from_braid(
            strands,
            &word.map_err(|_| "braid: bad generator".to_string())?,
        )
    } else {
        Diagram::from_pd(s)
    }
}

fn flag_usize(args: &[String], name: &str, default: usize) -> Result<usize, String> {
    match args.iter().position(|a| a == name) {
        Some(i) => args
            .get(i + 1)
            .ok_or(format!("{} needs a value", name))?
            .parse()
            .map_err(|_| format!("{} needs a number", name)),
        None => Ok(default),
    }
}

fn flag_str<'a>(args: &'a [String], name: &str, default: &'a str) -> &'a str {
    match args.iter().position(|a| a == name) {
        Some(i) => args.get(i + 1).map(|s| s.as_str()).unwrap_or(default),
        None => default,
    }
}

fn positional(args: &[String]) -> Result<&String, String> {
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if a.starts_with("--") {
            skip = true;
            continue;
        }
        return Ok(a);
    }
    Err("missing input".into())
}

fn cmd_verify(args: &[String]) -> Result<(), String> {
    if args.is_empty() {
        return Err("verify: no files given".into());
    }
    let mut failed = 0;
    for path in args {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
        match Cert::parse(&text).and_then(|c| c.verify()) {
            Ok(r) => {
                let value = r.value.map(|v| v.to_string()).unwrap_or("-".into());
                println!(
                    "ok    {}  [{} value={} cost={} steps={}]",
                    path, r.claim, value, r.cost, r.steps
                );
                for w in r.warnings {
                    println!("      warning: {}", w);
                }
            }
            Err(e) => {
                failed += 1;
                println!("FAIL  {}\n      {}", path, e.replace('\n', "\n      "));
            }
        }
    }
    if failed > 0 {
        return Err(format!("{} certificate(s) failed", failed));
    }
    Ok(())
}

/// Rust's runtime ignores SIGPIPE so that writing to a closed pipe returns an
/// error instead of killing the process — which turns `unknotdb info K | head`
/// into a panic message. Every other Unix tool dies quietly there, and callers
/// expect the conventional 141 exit status, so restore the default handler.
///
/// `signal` lives in libc, which std already links, so declaring it here keeps
/// the crate dependency-free. SIGPIPE is 13 and SIG_DFL is 0 on Linux, macOS
/// and the BSDs alike.
#[cfg(unix)]
fn restore_sigpipe_default() {
    const SIGPIPE: i32 = 13;
    const SIG_DFL: usize = 0;
    extern "C" {
        fn signal(signum: i32, handler: usize) -> usize;
    }
    unsafe {
        signal(SIGPIPE, SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_sigpipe_default() {}

fn main() {
    restore_sigpipe_default();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        print!("{}", USAGE);
        std::process::exit(2);
    }
    let rest = &args[2..];
    let result = (|| -> Result<(), String> {
        match args[1].as_str() {
            "help" | "--help" | "-h" => {
                print!("{}", USAGE);
                Ok(())
            }
            "verify" => cmd_verify(rest),
            "canon" => {
                let d = parse_input(positional(rest)?)?;
                println!("{}", d.canon(true));
                Ok(())
            }
            "pd" => {
                let d = parse_input(positional(rest)?)?;
                println!("{}", d.to_pd());
                Ok(())
            }
            "info" => {
                let d = parse_input(positional(rest)?)?;
                println!("crossings:  {}", d.n);
                println!("faces:      {}", d.faces().len());
                println!("components: {}", d.components());
                println!("writhe:     {}", d.writhe());
                println!("canon:      {}", d.canon(true));
                println!("pd:         {}", d.to_pd());
                Ok(())
            }
            "tri" => {
                let d = parse_input(positional(rest)?)?;
                for (c1, c2, c3) in d.triangles() {
                    println!("R3 c1={} c2={} c3={}", c1, c2, c3);
                }
                Ok(())
            }
            "reduce" => {
                let d = parse_input(positional(rest)?)?;
                let (red, trace) = d.reduce();
                for m in &trace {
                    println!("{}", m);
                }
                eprintln!("# {} moves, {} crossings remain", trace.len(), red.n);
                Ok(())
            }
            "unknot" => {
                let d = parse_input(positional(rest)?)?;
                let max_u = flag_usize(rest, "--max-u", 4)?;
                let cap = flag_usize(rest, "--cap", 500_000)?;
                let r3 = flag_usize(rest, "--r3", 2)?;
                match unknotting_trace(&d, max_u, cap, r3) {
                    Some((cost, trace)) => {
                        for m in &trace {
                            println!("{}", m);
                        }
                        eprintln!("# u <= {} ({} moves)", cost, trace.len());
                        Ok(())
                    }
                    None => Err(format!(
                        "no unknotting trace found with u <= {}. The search only \
                         reduces (R1-, R2-, R3, XC); it never uses R2+, so \
                         diagrams that must first grow are out of its reach. \
                         The verifier does accept R2+ traces from other producers.",
                        max_u
                    )),
                }
            }
            "mkcert" => {
                let d = parse_input(positional(rest)?)?;
                let max_u = flag_usize(rest, "--max-u", 4)?;
                let cap = flag_usize(rest, "--cap", 500_000)?;
                let r3 = flag_usize(rest, "--r3", 2)?;
                let (cost, trace) = unknotting_trace(&d, max_u, cap, r3)
                    .ok_or_else(|| format!("no unknotting trace with u <= {}", max_u))?;
                let knotinfo = flag_str(rest, "--knotinfo", "");
                print!(
                    "{}",
                    cert::emit(
                        "unknotting_number_le",
                        cost,
                        &d,
                        &trace,
                        if knotinfo.is_empty() {
                            None
                        } else {
                            Some(knotinfo)
                        },
                        flag_str(rest, "--source", "this repository"),
                        &format!("unknotdb/{} search", env!("CARGO_PKG_VERSION")),
                        flag_str(rest, "--date", "unset"),
                    )
                );
                Ok(())
            }
            "id" => {
                let path = positional(rest)?;
                let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {}", path, e))?;
                let c = Cert::parse(&text)?;
                let canon = c
                    .fields
                    .get("subject.canon")
                    .ok_or("certificate has no subject.canon")?;
                let claim = c.fields.get("claim").ok_or("certificate has no claim")?;
                let h = util::sha256_hex(canon.as_bytes());
                println!("certs/{}/{}.cert", claim, &h[..16]);
                Ok(())
            }
            other => Err(format!("unknown command `{}` (try `unknotdb help`)", other)),
        }
    })();

    if let Err(e) = result {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

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
