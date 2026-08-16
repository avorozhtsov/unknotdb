use unknotdb::cert::{self, Cert};
use unknotdb::diagram::Diagram;
use unknotdb::search::unknotting_trace;
use unknotdb::util;

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
