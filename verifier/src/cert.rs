//! Certificate parsing and verification.
//!
//! The frontmatter parser is deliberately small and strict: a flat key/value
//! block with one level of nesting. It is not YAML and does not try to be.

use crate::diagram::Diagram;
use crate::moves::Move;
use std::collections::BTreeMap;

pub struct Cert {
    pub fields: BTreeMap<String, String>,
    pub trace: Vec<Move>,
}

pub struct Report {
    pub claim: String,
    pub cost: usize,
    pub value: Option<usize>,
    pub steps: usize,
    pub warnings: Vec<String>,
}

const REQUIRED: [&str; 7] = [
    "claim",
    "alphabet",
    "chirality",
    "assumes",
    "claim_source",
    "witness_author",
    "witness_date",
];

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2
        && ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

impl Cert {
    pub fn parse(text: &str) -> Result<Cert, String> {
        let mut lines = text.lines();
        match lines.next().map(str::trim) {
            Some("---") => {}
            _ => return Err("certificate must start with `---`".into()),
        }
        let mut fields: BTreeMap<String, String> = BTreeMap::new();
        let mut prefix: Option<String> = None;
        let mut closed = false;
        let mut rest: Vec<&str> = Vec::new();

        for line in lines.by_ref() {
            if line.trim() == "---" {
                closed = true;
                break;
            }
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let indented = line.starts_with(' ') || line.starts_with('\t');
            let (k, v) = line
                .split_once(':')
                .ok_or_else(|| format!("frontmatter: expected `key: value`, got `{}`", line))?;
            let key = k.trim().to_string();
            let val = unquote(v);
            if indented {
                let p = prefix.clone().ok_or_else(|| {
                    format!("frontmatter: `{}` is indented but has no parent", key)
                })?;
                fields.insert(format!("{}.{}", p, key), val);
            } else if val.is_empty() {
                prefix = Some(key);
            } else {
                prefix = None;
                fields.insert(key, val);
            }
        }
        if !closed {
            return Err("frontmatter is not terminated by `---`".into());
        }
        for line in lines {
            rest.push(line);
        }

        let mut trace = Vec::new();
        for (i, line) in rest.iter().enumerate() {
            let stripped = line.split('#').next().unwrap_or("").trim();
            if stripped.is_empty() {
                continue;
            }
            trace.push(Move::parse(stripped).map_err(|e| format!("trace line {}: {}", i + 1, e))?);
        }
        Ok(Cert { fields, trace })
    }

    fn get(&self, k: &str) -> Option<&str> {
        self.fields.get(k).map(|s| s.as_str())
    }

    fn require(&self, k: &str) -> Result<&str, String> {
        self.get(k)
            .ok_or_else(|| format!("missing required field `{}`", k))
    }

    /// `u` and friends are mirror-invariant; `equivalent` is not.
    fn mirror_invariant(claim: &str) -> bool {
        matches!(
            claim,
            "unknot" | "unknotting_number_le" | "band_unknotting_le" | "slice_genus_le"
        )
    }

    pub fn verify(&self) -> Result<Report, String> {
        for k in REQUIRED {
            self.require(k)?;
        }
        let claim = self.require("claim")?.to_string();
        let chirality = self.require("chirality")?;
        if chirality != "unknotdb" {
            return Err(format!(
                "chirality is `{}`; only `unknotdb` is accepted (see schema/conventions.md §2)",
                chirality
            ));
        }
        let alphabet = self.require("alphabet")?.to_string();
        match alphabet.as_str() {
            "R" | "X" => {}
            "M" | "B" => {
                return Err(format!(
                    "alphabet `{}` is specified but not implemented in v0; refusing to accept the certificate",
                    alphabet
                ))
            }
            other => return Err(format!("unknown alphabet `{}`", other)),
        }
        let value = match self.get("value") {
            Some(v) => Some(
                v.parse::<usize>()
                    .map_err(|_| format!("`value: {}` is not a non-negative integer", v))?,
            ),
            None => None,
        };
        if claim.ends_with("_le") && value.is_none() {
            return Err(format!("claim `{}` requires a `value`", claim));
        }

        let pd = self.require("subject.pd")?;
        let mut d = Diagram::from_pd(pd).map_err(|e| format!("subject.pd: {}", e))?;
        let mut warnings = Vec::new();

        let mirror_inv = Cert::mirror_invariant(&claim);
        if let Some(stated) = self.get("subject.canon") {
            let computed = d.canon(mirror_inv);
            if stated != computed {
                return Err(format!(
                    "subject.canon does not match the diagram\n  stated:   {}\n  computed: {}",
                    stated, computed
                ));
            }
        } else {
            warnings.push("subject.canon absent; the record is not keyed".to_string());
        }

        for (i, m) in self.trace.iter().enumerate() {
            if alphabet == "R" && m.alphabet() != 'R' {
                return Err(format!(
                    "move {} (`{}`) is outside the declared alphabet `R`",
                    i + 1,
                    m
                ));
            }
            d = d.apply(*m).map_err(|e| {
                format!(
                    "move {} (`{}`) rejected: {}\n  state: {} crossings",
                    i + 1,
                    m,
                    e,
                    d.n
                )
            })?;
        }

        let cost = self.trace.iter().filter(|m| m.is_crossing_change()).count();

        match claim.as_str() {
            "unknot" | "unknotting_number_le" => {
                if d.n != 0 {
                    return Err(format!("trace ends with {} crossings, not the unknot", d.n));
                }
                if d.free_loops != 1 {
                    return Err(format!(
                        "trace ends with {} components, expected 1",
                        d.free_loops
                    ));
                }
            }
            "equivalent" => {
                let target = self.require("target.pd")?;
                let t = Diagram::from_pd(target).map_err(|e| format!("target.pd: {}", e))?;
                if d.canon(false) != t.canon(false) {
                    return Err("trace does not end at the stated target diagram".to_string());
                }
            }
            other => {
                return Err(format!(
                    "claim `{}` is defined in the schema but has no verifier in v0",
                    other
                ))
            }
        }

        if let Some(v) = value {
            if cost > v {
                return Err(format!("trace costs {} but the claim states {}", cost, v));
            }
            if cost < v {
                warnings.push(format!(
                    "trace costs {}, beating the stated value {}; file a tighter certificate",
                    cost, v
                ));
            }
        }
        let assumes = self.require("assumes")?;
        if assumes != "[]" {
            warnings.push(format!("conditional claim: assumes {}", assumes));
        }

        Ok(Report {
            claim,
            cost,
            value,
            steps: self.trace.len(),
            warnings,
        })
    }
}

/// Emit a certificate for an unknotting trace produced by the search.
pub fn emit(
    claim: &str,
    value: usize,
    d: &Diagram,
    trace: &[Move],
    knotinfo: Option<&str>,
    source: &str,
    author: &str,
    date: &str,
) -> String {
    let mut s = String::new();
    s.push_str("---\n");
    s.push_str(&format!("claim: {}\n", claim));
    s.push_str(&format!("value: {}\n", value));
    s.push_str("subject:\n");
    s.push_str(&format!(
        "  canon: \"{}\"\n",
        d.canon(Cert::mirror_invariant(claim))
    ));
    s.push_str(&format!("  pd: \"{}\"\n", d.to_pd()));
    if let Some(k) = knotinfo {
        s.push_str(&format!("  knotinfo: \"{}\"\n", k));
    }
    s.push_str("alphabet: X\n");
    s.push_str("chirality: unknotdb\n");
    s.push_str("assumes: []\n");
    s.push_str(&format!("claim_source: \"{}\"\n", source));
    s.push_str(&format!("witness_author: \"{}\"\n", author));
    s.push_str(&format!("witness_date: {}\n", date));
    s.push_str("---\n");
    for m in trace {
        s.push_str(&format!("{}\n", m));
    }
    s
}
