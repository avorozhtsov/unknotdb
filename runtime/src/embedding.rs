use crate::representation::{
    AnchoredProofProgram, BraidRepresentation, CheckpointedProofProgram, ProofInstruction,
    SemanticAction,
};
use crate::Result;
use rusqlite::{params, Connection, OpenFlags};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA: &str = "unknotdb-embedding-pairs-v3";

#[derive(Clone, Debug)]
struct RepresentationRow {
    sha256: [u8; 32],
    encoding: Vec<u8>,
    origin_key: [u8; 32],
    mirror_key: [u8; 32],
    strands: u16,
    word_length: u32,
    knot_id: Option<String>,
}

#[derive(Clone, Debug, Default)]
struct RepresentationDomain {
    stopping_point: bool,
    edge_interior: bool,
    mirror_image: bool,
}

#[derive(Clone, Debug)]
struct PairRow {
    metric: &'static str,
    left: u32,
    right: u32,
    lower: u8,
    upper: u8,
    provenance: &'static str,
    edge_id: u32,
    instruction_index: u32,
    witness_semantic_length: u32,
}

#[derive(Clone, Debug)]
pub struct EmbeddingPairReport {
    pub replayed_edges: usize,
    pub representations: usize,
    pub pairs: usize,
    pub exact_cc0: usize,
    pub exact_cc1: usize,
    pub upper_cc1: usize,
    pub exact_rm0: usize,
    pub exact_rm1: usize,
    pub exact_rm2: usize,
    pub split_train: usize,
    pub split_validation: usize,
    pub split_test: usize,
    pub bytes: u64,
    pub proof_sha256: String,
    pub identification_sha256: String,
}

#[derive(Clone, Debug)]
struct UnionFind {
    parent: Vec<u32>,
}

impl UnionFind {
    fn new(size: usize) -> Result<Self> {
        Ok(Self {
            parent: (0..size)
                .map(u32::try_from)
                .collect::<std::result::Result<_, _>>()?,
        })
    }

    fn root(&mut self, value: u32) -> u32 {
        let parent = self.parent[value as usize];
        if parent != value {
            let root = self.root(parent);
            self.parent[value as usize] = root;
        }
        self.parent[value as usize]
    }

    fn union(&mut self, left: u32, right: u32) {
        let left = self.root(left);
        let right = self.root(right);
        if left != right {
            let (small, large) = if left < right {
                (left, right)
            } else {
                (right, left)
            };
            self.parent[large as usize] = small;
        }
    }
}

fn temporary_path(output: &Path) -> Result<PathBuf> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("embedding sidecar path has no UTF-8 file name")?;
    Ok(parent.join(format!(".{name}.{}.part", std::process::id())))
}

fn origin_key(representation: &BraidRepresentation) -> Result<[u8; 32]> {
    Ok(unknotdb::util::sha256(
        &representation.normalize_origin()?.encoded,
    ))
}

fn add_representation(
    representation: &BraidRepresentation,
    knot_ids: &HashMap<[u8; 32], String>,
    rows: &mut Vec<RepresentationRow>,
    ids: &mut HashMap<[u8; 32], u32>,
) -> Result<u32> {
    let encoding = representation.encode()?;
    let sha256 = unknotdb::util::sha256(&encoding);
    if let Some(&id) = ids.get(&sha256) {
        return Ok(id);
    }
    let mirror_key = representation.normalize()?.key;
    let id = u32::try_from(rows.len())?;
    rows.push(RepresentationRow {
        sha256,
        encoding,
        origin_key: origin_key(representation)?,
        mirror_key,
        strands: representation.strands,
        word_length: representation.word.len().try_into()?,
        knot_id: knot_ids.get(&mirror_key).cloned(),
    });
    ids.insert(sha256, id);
    Ok(id)
}

fn add_pair(
    pairs: &mut BTreeMap<(&'static str, u32, u32), PairRow>,
    mut pair: PairRow,
) -> Result<()> {
    if pair.left == pair.right {
        pair.lower = 0;
        pair.upper = 0;
    }
    if pair.left > pair.right {
        std::mem::swap(&mut pair.left, &mut pair.right);
    }
    if pair.lower > pair.upper {
        return Err("embedding pair has an empty distance interval".into());
    }
    let key = (pair.metric, pair.left, pair.right);
    if let Some(existing) = pairs.get_mut(&key) {
        existing.lower = existing.lower.max(pair.lower);
        existing.upper = existing.upper.min(pair.upper);
        if existing.lower > existing.upper {
            return Err("inconsistent embedding pair intervals".into());
        }
        if pair.witness_semantic_length < existing.witness_semantic_length {
            existing.provenance = pair.provenance;
            existing.edge_id = pair.edge_id;
            existing.instruction_index = pair.instruction_index;
            existing.witness_semantic_length = pair.witness_semantic_length;
        }
    } else {
        pairs.insert(key, pair);
    }
    Ok(())
}

fn zero_semantic(action: SemanticAction) -> bool {
    !matches!(
        action,
        SemanticAction::CrossingChange { .. }
            | SemanticAction::Pass
            | SemanticAction::DescendingCollapse
            | SemanticAction::PlanarCertificateCollapse
    )
}

fn one_zero_move_reaches(
    source: &BraidRepresentation,
    target_origin_key: [u8; 32],
) -> Result<bool> {
    let len = source.word.len();
    let mut candidates = Vec::new();
    for position in 0..len {
        let position = u32::try_from(position)?;
        candidates.push(SemanticAction::Reduce { position });
        candidates.push(SemanticAction::Commute { position });
        candidates.push(SemanticAction::Braid { position });
        let largest = if source.cyclic_band_generators {
            source.strands
        } else {
            source.strands.saturating_sub(1)
        };
        for generator in 1..=largest {
            for sign in [-1, 1] {
                candidates.push(SemanticAction::Insert {
                    position,
                    generator,
                    sign,
                });
            }
        }
    }
    if len == 0 {
        for generator in 1..source.strands {
            for sign in [-1, 1] {
                candidates.push(SemanticAction::Insert {
                    position: 0,
                    generator,
                    sign,
                });
            }
        }
    }
    candidates.push(SemanticAction::Destabilize);
    for position in 0..=len {
        for sign in [-1, 1] {
            candidates.push(SemanticAction::StabilizeAt {
                position: u32::try_from(position)?,
                sign,
            });
        }
    }
    for action in candidates {
        if let Ok(after) = action.apply(source) {
            if origin_key(&after)? == target_origin_key {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn split(unit: &[u8]) -> &'static str {
    let first = unknotdb::util::sha256(unit)[0];
    match first {
        0..=204 => "train",
        205..=229 => "validation",
        _ => "test",
    }
}

pub fn export_embedding_pairs(
    proof: &Path,
    identification: &Path,
    output: &Path,
) -> Result<EmbeddingPairReport> {
    if output.exists() {
        return Err(format!("embedding sidecar already exists: {}", output.display()).into());
    }
    let proof_sha256 = unknotdb::util::sha256_hex(&fs::read(proof)?);
    let identification_sha256 = unknotdb::util::sha256_hex(&fs::read(identification)?);
    let proof_db = Connection::open_with_flags(proof, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let identification_db =
        Connection::open_with_flags(identification, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let pinned: String = identification_db.query_row(
        "SELECT value FROM meta WHERE key='graph_snapshot_sha256'",
        [],
        |row| row.get(0),
    )?;
    if pinned != proof_sha256 {
        return Err("identification sidecar is pinned to another proof snapshot".into());
    }
    let mut knot_ids = HashMap::new();
    let mut knot_rows = identification_db
        .prepare("SELECT rep_key,knot_id FROM graph_vertex_knot_map ORDER BY rep_key")?;
    for row in knot_rows.query_map([], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
    })? {
        let (key, knot_id) = row?;
        knot_ids.insert(
            key.try_into().map_err(|_| "bad identification rep key")?,
            knot_id,
        );
    }

    let mut representations = Vec::new();
    let mut representation_ids = HashMap::new();
    let mut representation_domains = Vec::<RepresentationDomain>::new();
    let mut pairs = BTreeMap::new();
    let mut cc_graph_edges = Vec::<(u32, u32, u32, u32, u32, u32)>::new();
    let mut replayed_edges = 0usize;
    let mut edges = proof_db.prepare(
        "SELECT e.edge_id,e.source_node,e.target_node,e.cc_cost,e.program_anchor_x, \
                e.program_anchor_y,p.program_version,p.program,s.encoding,t.encoding \
         FROM edges e JOIN programs p USING(program_id) \
         JOIN representations s ON s.node_id=e.source_node \
         JOIN representations t ON t.node_id=e.target_node ORDER BY e.edge_id",
    )?;
    let rows = edges.query_map([], |row| {
        Ok((
            row.get::<_, u32>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, u32>(2)?,
            row.get::<_, u8>(3)?,
            row.get::<_, u16>(4)?,
            row.get::<_, u32>(5)?,
            row.get::<_, u32>(6)?,
            row.get::<_, Vec<u8>>(7)?,
            row.get::<_, Vec<u8>>(8)?,
            row.get::<_, Vec<u8>>(9)?,
        ))
    })?;
    for row in rows {
        let (
            edge_id,
            source_node,
            target_node,
            cc_cost,
            anchor_x,
            anchor_y,
            version,
            bytes,
            source_bytes,
            target_bytes,
        ) = row?;
        if version != CheckpointedProofProgram::VERSION {
            return Err(format!("unsupported embedding program version {version}").into());
        }
        let source = BraidRepresentation::decode_storage(&source_bytes)?;
        let target = BraidRepresentation::decode_storage(&target_bytes)?;
        let template = CheckpointedProofProgram::decode(&bytes)?;
        if template.contains_planar_certificate() {
            continue;
        }
        let program = AnchoredProofProgram {
            template,
            anchor_x,
            anchor_y,
        }
        .materialize(&source)?;
        if program.cc_cost() != u32::from(cc_cost) || program.replay(&source)? != target {
            return Err(format!("embedding edge {edge_id} failed replay").into());
        }
        let semantic_length = u32::try_from(program.semantic_replay_steps(&source)?.len())?;
        replayed_edges += 1;
        let source_representation = add_representation(
            &source,
            &knot_ids,
            &mut representations,
            &mut representation_ids,
        )?;
        representation_domains.resize_with(representations.len(), Default::default);
        representation_domains[source_representation as usize].stopping_point = true;
        let target_representation = add_representation(
            &target,
            &knot_ids,
            &mut representations,
            &mut representation_ids,
        )?;
        representation_domains.resize_with(representations.len(), Default::default);
        representation_domains[target_representation as usize].stopping_point = true;
        if cc_cost == 0 {
            add_pair(
                &mut pairs,
                PairRow {
                    metric: "cc",
                    left: source_representation,
                    right: target_representation,
                    lower: 0,
                    upper: 0,
                    provenance: "replayed_zero_cc_graph_edge",
                    edge_id,
                    instruction_index: 0,
                    witness_semantic_length: semantic_length,
                },
            )?;
        } else {
            let left_knot = representations[source_representation as usize]
                .knot_id
                .as_deref();
            let right_knot = representations[target_representation as usize]
                .knot_id
                .as_deref();
            let lower =
                u8::from(left_knot.is_some() && right_knot.is_some() && left_knot != right_knot);
            add_pair(
                &mut pairs,
                PairRow {
                    metric: "cc",
                    left: source_representation,
                    right: target_representation,
                    lower,
                    upper: 1,
                    provenance: if lower == 1 {
                        "graph_edge_distinct_attested_knots"
                    } else {
                        "replayed_graph_edge_upper_bound"
                    },
                    edge_id,
                    instruction_index: 0,
                    witness_semantic_length: semantic_length,
                },
            )?;
            cc_graph_edges.push((
                source_node,
                target_node,
                source_representation,
                target_representation,
                edge_id,
                semantic_length,
            ));
        }
        let steps = program.instruction_replay_steps(&source)?;
        let mut previous_zero: Option<(u32, BraidRepresentation)> = None;
        for step in steps {
            let before = add_representation(
                &step.before,
                &knot_ids,
                &mut representations,
                &mut representation_ids,
            )?;
            let after = add_representation(
                &step.after,
                &knot_ids,
                &mut representations,
                &mut representation_ids,
            )?;
            representation_domains.resize_with(representations.len(), Default::default);
            representation_domains[before as usize].edge_interior = true;
            representation_domains[after as usize].edge_interior = true;
            match step.instruction {
                ProofInstruction::NormalizeOrigin { .. }
                | ProofInstruction::RotateOriginLeft { .. } => {
                    for metric in ["cc", "rm"] {
                        add_pair(
                            &mut pairs,
                            PairRow {
                                metric,
                                left: before,
                                right: after,
                                lower: 0,
                                upper: 0,
                                provenance: "coordinate_replay",
                                edge_id,
                                instruction_index: step.instruction_index,
                                witness_semantic_length: 0,
                            },
                        )?;
                    }
                }
                ProofInstruction::MirrorOrbit => {
                    representation_domains[after as usize].mirror_image = true;
                    add_pair(
                        &mut pairs,
                        PairRow {
                            metric: "cc",
                            left: before,
                            right: after,
                            lower: 0,
                            upper: 0,
                            provenance: "mirror_orbit_task_symmetry",
                            edge_id,
                            instruction_index: step.instruction_index,
                            witness_semantic_length: 0,
                        },
                    )?;
                    previous_zero = None;
                }
                ProofInstruction::Action(action) if zero_semantic(action) => {
                    add_pair(
                        &mut pairs,
                        PairRow {
                            metric: "cc",
                            left: before,
                            right: after,
                            lower: 0,
                            upper: 0,
                            provenance: "replayed_zero_cc_action",
                            edge_id,
                            instruction_index: step.instruction_index,
                            witness_semantic_length: 1,
                        },
                    )?;
                    let lower = u8::from(
                        representations[before as usize].origin_key
                            != representations[after as usize].origin_key,
                    );
                    add_pair(
                        &mut pairs,
                        PairRow {
                            metric: "rm",
                            left: before,
                            right: after,
                            lower,
                            upper: lower,
                            provenance: "replayed_exact_zero_primitive",
                            edge_id,
                            instruction_index: step.instruction_index,
                            witness_semantic_length: 1,
                        },
                    )?;
                    if let Some((first, first_state)) = previous_zero.take() {
                        let target_key = representations[after as usize].origin_key;
                        let lower = if origin_key(&first_state)? == target_key {
                            0
                        } else if one_zero_move_reaches(&first_state, target_key)? {
                            1
                        } else {
                            2
                        };
                        add_pair(
                            &mut pairs,
                            PairRow {
                                metric: "rm",
                                left: first,
                                right: after,
                                lower,
                                upper: 2,
                                provenance: "bounded_exact_two_primitive",
                                edge_id,
                                instruction_index: step.instruction_index,
                                witness_semantic_length: 2,
                            },
                        )?;
                    }
                    previous_zero = Some((before, step.before));
                }
                ProofInstruction::Action(SemanticAction::CrossingChange { .. }) => {
                    let left_knot = representations[before as usize].knot_id.as_deref();
                    let right_knot = representations[after as usize].knot_id.as_deref();
                    let lower = u8::from(
                        left_knot.is_some() && right_knot.is_some() && left_knot != right_knot,
                    );
                    add_pair(
                        &mut pairs,
                        PairRow {
                            metric: "cc",
                            left: before,
                            right: after,
                            lower,
                            upper: 1,
                            provenance: if lower == 1 {
                                "replayed_cc_distinct_attested_knots"
                            } else {
                                "replayed_cc_upper_bound"
                            },
                            edge_id,
                            instruction_index: step.instruction_index,
                            witness_semantic_length: 1,
                        },
                    )?;
                    previous_zero = None;
                }
                ProofInstruction::Action(_) => previous_zero = None,
            }
        }
    }

    // A bounded deterministic supply of uncertain two-CC pairs. These are
    // upper bounds only unless distinct attested knot IDs supply lower >= 1.
    let mut outgoing = HashMap::<u32, Vec<(u32, u32, u32, u32)>>::new();
    for &(source_node, target_node, _source_rep, target_rep, edge_id, semantic_length) in
        &cc_graph_edges
    {
        outgoing.entry(source_node).or_default().push((
            target_node,
            target_rep,
            edge_id,
            semantic_length,
        ));
    }
    let mut two_cc_pairs = 0usize;
    for &(_source_node, middle_node, source_rep, _middle_rep, first_edge, first_length) in
        &cc_graph_edges
    {
        let Some(second_edges) = outgoing.get(&middle_node) else {
            continue;
        };
        for &(_target_node, target_rep, _second_edge, second_length) in second_edges.iter().take(4)
        {
            if source_rep == target_rep {
                continue;
            }
            let left_knot = representations[source_rep as usize].knot_id.as_deref();
            let right_knot = representations[target_rep as usize].knot_id.as_deref();
            let lower =
                u8::from(left_knot.is_some() && right_knot.is_some() && left_knot != right_knot);
            add_pair(
                &mut pairs,
                PairRow {
                    metric: "cc",
                    left: source_rep,
                    right: target_rep,
                    lower,
                    upper: 2,
                    provenance: "replayed_two_cc_graph_path_upper_bound",
                    edge_id: first_edge,
                    instruction_index: 0,
                    witness_semantic_length: first_length.saturating_add(second_length),
                },
            )?;
            two_cc_pairs += 1;
            if two_cc_pairs >= 50_000 {
                break;
            }
        }
        if two_cc_pairs >= 50_000 {
            break;
        }
    }

    let mut union = UnionFind::new(representations.len())?;
    for pair in pairs.values() {
        if pair.metric == "cc" && pair.lower == 0 && pair.upper == 0 {
            union.union(pair.left, pair.right);
        }
    }
    let mut by_knot: HashMap<String, u32> = HashMap::new();
    for (index, row) in representations.iter().enumerate() {
        if let Some(knot_id) = &row.knot_id {
            let index = u32::try_from(index)?;
            if let Some(&other) = by_knot.get(knot_id) {
                union.union(index, other);
            } else {
                by_knot.insert(knot_id.clone(), index);
            }
        }
    }
    let mut component_unit = HashMap::<u32, Vec<u8>>::new();
    for (index, row) in representations.iter().enumerate() {
        let root = union.root(u32::try_from(index)?);
        let candidate = row
            .knot_id
            .as_ref()
            .map(|value| format!("knot:{value}").into_bytes())
            .unwrap_or_else(|| row.mirror_key.to_vec());
        component_unit
            .entry(root)
            .and_modify(|unit| {
                if candidate < *unit {
                    *unit = candidate.clone();
                }
            })
            .or_insert(candidate);
    }
    let representation_splits: Vec<_> = (0..representations.len())
        .map(|index| {
            let root = union.root(u32::try_from(index).expect("representation count fits u32"));
            split(&component_unit[&root])
        })
        .collect();
    let cross_split_pairs: Vec<_> = pairs
        .values()
        .filter(|pair| {
            representation_splits[pair.left as usize] != representation_splits[pair.right as usize]
        })
        .cloned()
        .collect();
    pairs.retain(|_, pair| {
        representation_splits[pair.left as usize] == representation_splits[pair.right as usize]
    });

    let temporary = temporary_path(output)?;
    let mut db = Connection::open(&temporary)?;
    db.execute_batch(
        r#"
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        PRAGMA user_version=3;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE representations(
          representation_id INTEGER PRIMARY KEY,
          sha256 BLOB NOT NULL UNIQUE CHECK(length(sha256)=32),
          encoding BLOB NOT NULL,
          origin_key BLOB NOT NULL CHECK(length(origin_key)=32),
          mirror_key BLOB NOT NULL CHECK(length(mirror_key)=32),
          strands INTEGER NOT NULL,
          word_length INTEGER NOT NULL,
          knot_id TEXT,
          split TEXT NOT NULL CHECK(split IN ('train','validation','test'))
        );
        CREATE INDEX representations_by_split ON representations(split,representation_id);
        CREATE TABLE representation_domains(
          representation_id INTEGER PRIMARY KEY REFERENCES representations(representation_id),
          is_stopping_point INTEGER NOT NULL CHECK(is_stopping_point IN (0,1)),
          is_edge_interior INTEGER NOT NULL CHECK(is_edge_interior IN (0,1)),
          is_mirror_image INTEGER NOT NULL CHECK(is_mirror_image IN (0,1))
        ) WITHOUT ROWID;
        CREATE INDEX representation_domains_by_role ON representation_domains(
          is_stopping_point,is_edge_interior,is_mirror_image,representation_id
        );
        CREATE TABLE pairs(
          pair_id INTEGER PRIMARY KEY,
          metric TEXT NOT NULL CHECK(metric IN ('cc','rm')),
          left_representation INTEGER NOT NULL REFERENCES representations(representation_id),
          right_representation INTEGER NOT NULL REFERENCES representations(representation_id),
          distance_lower INTEGER NOT NULL CHECK(distance_lower BETWEEN 0 AND 2),
          distance_upper INTEGER NOT NULL CHECK(distance_upper BETWEEN 0 AND 2),
          exact INTEGER NOT NULL CHECK(exact IN (0,1)),
          provenance TEXT NOT NULL,
          edge_id INTEGER NOT NULL,
          instruction_index INTEGER NOT NULL,
          witness_semantic_length INTEGER NOT NULL CHECK(witness_semantic_length >= 0),
          split TEXT NOT NULL CHECK(split IN ('train','validation','test')),
          UNIQUE(metric,left_representation,right_representation)
        );
        CREATE INDEX pairs_by_metric_split ON pairs(metric,split,pair_id);
        CREATE TABLE excluded_cross_split_pairs(
          metric TEXT NOT NULL CHECK(metric IN ('cc','rm')),
          left_representation INTEGER NOT NULL REFERENCES representations(representation_id),
          right_representation INTEGER NOT NULL REFERENCES representations(representation_id),
          distance_lower INTEGER NOT NULL CHECK(distance_lower BETWEEN 0 AND 2),
          distance_upper INTEGER NOT NULL CHECK(distance_upper BETWEEN 0 AND 2),
          provenance TEXT NOT NULL,
          edge_id INTEGER NOT NULL,
          instruction_index INTEGER NOT NULL,
          witness_semantic_length INTEGER NOT NULL CHECK(witness_semantic_length >= 0),
          exclusion_reason TEXT NOT NULL,
          PRIMARY KEY(metric,left_representation,right_representation)
        ) WITHOUT ROWID;
        "#,
    )?;
    let transaction = db.transaction()?;
    for (key, value) in [
        ("schema", SCHEMA.to_owned()),
        ("proof_sha256", proof_sha256.clone()),
        ("identification_sha256", identification_sha256.clone()),
        ("cc_metric", "mirror-orbit task quotient; semantic CC count".to_owned()),
        ("rm_metric", "origin-quotiented reversible braid Reidemeister/Markov primitive count; mirror excluded".to_owned()),
        ("split_contract", "union exact CC=0 and attested knot IDs; hash unit; keep only within-split pairs".to_owned()),
        ("witness_length_contract", "number of replayed semantic actions in the shortest stored witness for the pair; coordinate and mirror instructions count zero".to_owned()),
        ("cross_split_contract", "excluded_cross_split_pairs is evaluation/provenance only and MUST NOT be sampled for training".to_owned()),
        ("domain_contract", "edge-interior states are exact instruction replay checkpoints; stopping-point and mirror-image roles may overlap".to_owned()),
    ] {
        transaction.execute("INSERT INTO meta VALUES (?1,?2)", params![key, value])?;
    }
    {
        let mut insert = transaction
            .prepare("INSERT INTO representations VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)")?;
        for (id, row) in representations.iter().enumerate() {
            insert.execute(params![
                u32::try_from(id)?,
                row.sha256.as_slice(),
                &row.encoding,
                row.origin_key.as_slice(),
                row.mirror_key.as_slice(),
                row.strands,
                row.word_length,
                row.knot_id,
                representation_splits[id],
            ])?;
        }
    }
    {
        let mut insert =
            transaction.prepare("INSERT INTO representation_domains VALUES (?1,?2,?3,?4)")?;
        for (id, domain) in representation_domains.iter().enumerate() {
            insert.execute(params![
                u32::try_from(id)?,
                u8::from(domain.stopping_point),
                u8::from(domain.edge_interior),
                u8::from(domain.mirror_image),
            ])?;
        }
    }
    {
        let mut insert = transaction.prepare(
            "INSERT INTO excluded_cross_split_pairs VALUES \
             (?1,?2,?3,?4,?5,?6,?7,?8,?9,'identity_or_orbit_split_boundary')",
        )?;
        for pair in &cross_split_pairs {
            insert.execute(params![
                pair.metric,
                pair.left,
                pair.right,
                pair.lower,
                pair.upper,
                pair.provenance,
                pair.edge_id,
                pair.instruction_index,
                pair.witness_semantic_length,
            ])?;
        }
    }
    {
        let mut insert = transaction
            .prepare("INSERT INTO pairs VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)")?;
        for (id, pair) in pairs.values().enumerate() {
            insert.execute(params![
                u32::try_from(id)?,
                pair.metric,
                pair.left,
                pair.right,
                pair.lower,
                pair.upper,
                u8::from(pair.lower == pair.upper),
                pair.provenance,
                pair.edge_id,
                pair.instruction_index,
                pair.witness_semantic_length,
                representation_splits[pair.left as usize],
            ])?;
        }
    }
    transaction.commit()?;
    db.execute_batch("PRAGMA optimize;")?;
    if db.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))? != "ok" {
        return Err("embedding pair sidecar integrity check failed".into());
    }
    drop(db);
    fs::rename(&temporary, output)?;
    let count = |metric: &str, lower: u8, upper: u8| {
        pairs
            .values()
            .filter(|pair| pair.metric == metric && pair.lower == lower && pair.upper == upper)
            .count()
    };
    Ok(EmbeddingPairReport {
        replayed_edges,
        representations: representations.len(),
        pairs: pairs.len(),
        exact_cc0: count("cc", 0, 0),
        exact_cc1: count("cc", 1, 1),
        upper_cc1: count("cc", 0, 1),
        exact_rm0: count("rm", 0, 0),
        exact_rm1: count("rm", 1, 1),
        exact_rm2: count("rm", 2, 2),
        split_train: pairs
            .values()
            .filter(|pair| representation_splits[pair.left as usize] == "train")
            .count(),
        split_validation: pairs
            .values()
            .filter(|pair| representation_splits[pair.left as usize] == "validation")
            .count(),
        split_test: pairs
            .values()
            .filter(|pair| representation_splits[pair.left as usize] == "test")
            .count(),
        bytes: fs::metadata(output)?.len(),
        proof_sha256,
        identification_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_one_move_check_finds_reduction_modulo_origin() {
        let source = BraidRepresentation {
            strands: 2,
            cyclic_band_generators: false,
            word: vec![1, -1, 1],
        };
        let target = SemanticAction::Reduce { position: 0 }
            .apply(&source)
            .unwrap();
        assert!(one_zero_move_reaches(&source, origin_key(&target).unwrap()).unwrap());
    }

    #[test]
    fn split_is_deterministic_and_total() {
        for value in [b"knot:3_1".as_slice(), b"component:deadbeef".as_slice()] {
            assert!(matches!(split(value), "train" | "validation" | "test"));
            assert_eq!(split(value), split(value));
        }
    }
}
