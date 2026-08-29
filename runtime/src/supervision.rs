use crate::representation::{
    AnchoredProofProgram, BraidRepresentation, CheckpointedProofProgram, SemanticAction,
};
use crate::Result;
use rusqlite::{params, Connection, OpenFlags};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

const SCHEMA: &str = "unknotdb-graph-supervision-v1";
const CC_FRONTIER_SCHEMA: &str = "unknotdb-cc-frontier-supervision-v0";

#[derive(Clone, Debug)]
struct Label {
    task: &'static str,
    state_sha256: [u8; 32],
    state_encoding: Vec<u8>,
    action: u64,
    action_kind: &'static str,
    strands: u16,
    word_length: u32,
    split: &'static str,
    occurrences: u32,
}

#[derive(Clone, Debug)]
struct Occurrence {
    label_id: u32,
    source_node: u32,
    edge_id: u32,
    program_id: u32,
    instruction_index: u32,
    phase: &'static str,
    route_min_cc: u32,
    route_semantic_moves: u64,
}

struct SelectedEdge {
    source: u32,
    target: u32,
    cc_cost: u8,
    program_id: u32,
    anchor_x: u16,
    anchor_y: u32,
    version: u32,
    program: Vec<u8>,
    source_representation: Vec<u8>,
    target_representation: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct SupervisionReport {
    pub selected_edges: usize,
    pub replayed_edges: usize,
    pub excluded_planar_edges: usize,
    pub excluded_theorem_actions: usize,
    pub preprocessor_labels: usize,
    pub cc_labels: usize,
    pub occurrences: usize,
    pub conflicting_states: usize,
    pub train_labels: usize,
    pub validation_labels: usize,
    pub test_labels: usize,
    pub bytes: u64,
    pub proof_sha256: String,
    pub routing_sha256: String,
}

#[derive(Clone, Debug)]
pub struct CcFrontierReport {
    pub candidate_edges: usize,
    pub replayed_edges: usize,
    pub states: usize,
    pub options: usize,
    pub accepted_options: usize,
    pub compared_options: usize,
    pub multi_accepted_states: usize,
    pub bytes: u64,
    pub proof_sha256: String,
    pub routing_sha256: String,
}

#[derive(Clone, Debug)]
struct CcOption {
    state_sha256: [u8; 32],
    state_encoding: Vec<u8>,
    action: u64,
    strands: u16,
    word_length: u32,
    split: &'static str,
    source_node: u32,
    edge_id: u32,
    target_node: u32,
    result_cc: u32,
    result_semantic_moves: u64,
    accepted: bool,
}

fn temporary_path(output: &Path) -> Result<PathBuf> {
    let parent = output.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let name = output
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("supervision sidecar path has no UTF-8 file name")?;
    Ok(parent.join(format!(".{name}.{}.part", std::process::id())))
}

fn action_kind(action: SemanticAction) -> &'static str {
    match action {
        SemanticAction::Reduce { .. } => "reduce",
        SemanticAction::Commute { .. } => "commute",
        SemanticAction::Braid { .. } => "braid",
        SemanticAction::Insert { .. } => "insert",
        SemanticAction::Destabilize => "destabilize",
        SemanticAction::StabilizePositive => "stabilize_positive",
        SemanticAction::StabilizeNegative => "stabilize_negative",
        SemanticAction::StabilizeAt { .. } => "stabilize_at",
        SemanticAction::Pass => "pass",
        SemanticAction::CrossingChange { .. } => "crossing_change",
        SemanticAction::DescendingCollapse => "descending_collapse",
        SemanticAction::PlanarCertificateCollapse => "planar_certificate_collapse",
    }
}

fn task(action: SemanticAction) -> Option<&'static str> {
    match action {
        SemanticAction::CrossingChange { .. } => Some("cc_solver"),
        SemanticAction::Pass
        | SemanticAction::DescendingCollapse
        | SemanticAction::PlanarCertificateCollapse => None,
        _ => Some("preprocessor"),
    }
}

fn split(hash: &[u8; 32]) -> &'static str {
    match hash[0] {
        0..=229 => "train",
        230..=242 => "validation",
        _ => "test",
    }
}

pub fn export_graph_supervision(
    proof: &Path,
    routing: &Path,
    output: &Path,
) -> Result<SupervisionReport> {
    if output.exists() {
        return Err(format!("supervision dataset already exists: {}", output.display()).into());
    }
    let proof_sha256 = unknotdb::util::sha256_hex(&fs::read(proof)?);
    let routing_sha256 = unknotdb::util::sha256_hex(&fs::read(routing)?);
    let proof_db = Connection::open_with_flags(proof, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let routing_db = Connection::open_with_flags(routing, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let pinned: String = routing_db.query_row(
        "SELECT value FROM meta WHERE key='proof_sha256'",
        [],
        |row| row.get(0),
    )?;
    if pinned != proof_sha256 {
        return Err("routing sidecar is pinned to a different proof snapshot".into());
    }

    let mut selected = routing_db.prepare(
        "SELECT node_id,next_shortest_edge,min_cc,shortest_at_min_cc_semantic_moves \
         FROM routes WHERE next_shortest_edge IS NOT NULL ORDER BY node_id",
    )?;
    let routes = selected.query_map([], |row| {
        Ok((
            row.get::<_, u32>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, u32>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;
    let mut edge = proof_db.prepare(
        "SELECT e.source_node,e.target_node,e.cc_cost,e.program_id, \
                e.program_anchor_x,e.program_anchor_y,p.program_version,p.program, \
                sr.encoding,tr.encoding \
         FROM edges e JOIN programs p USING(program_id) \
         JOIN representations sr ON sr.node_id=e.source_node \
         JOIN representations tr ON tr.node_id=e.target_node \
         WHERE e.edge_id=?1",
    )?;

    let mut labels: Vec<Label> = Vec::new();
    let mut label_ids: BTreeMap<(u8, [u8; 32], u64), u32> = BTreeMap::new();
    let mut occurrences = Vec::new();
    let mut state_actions: BTreeMap<(u8, [u8; 32]), BTreeSet<u64>> = BTreeMap::new();
    let mut selected_edges = 0;
    let mut replayed_edges = 0;
    let mut excluded_planar_edges = 0;
    let mut excluded_theorem_actions = 0;

    for route in routes {
        let (source_node, edge_id, route_min_cc, route_semantic_moves) = route?;
        let route_semantic_moves: u64 = route_semantic_moves.try_into()?;
        selected_edges += 1;
        let selected_edge = edge.query_row([edge_id], |row| {
            Ok(SelectedEdge {
                source: row.get(0)?,
                target: row.get(1)?,
                cc_cost: row.get(2)?,
                program_id: row.get(3)?,
                anchor_x: row.get(4)?,
                anchor_y: row.get(5)?,
                version: row.get(6)?,
                program: row.get(7)?,
                source_representation: row.get(8)?,
                target_representation: row.get(9)?,
            })
        })?;
        if selected_edge.source != source_node || selected_edge.target == source_node {
            return Err("routing edge has inconsistent proof endpoints".into());
        }
        if selected_edge.version != CheckpointedProofProgram::VERSION {
            return Err(format!(
                "unsupported supervision program version {}",
                selected_edge.version
            )
            .into());
        }
        let source = BraidRepresentation::decode_storage(&selected_edge.source_representation)?;
        let target = BraidRepresentation::decode_storage(&selected_edge.target_representation)?;
        let template = CheckpointedProofProgram::decode(&selected_edge.program)?;
        if template.contains_planar_certificate() {
            excluded_planar_edges += 1;
            continue;
        }
        let program = AnchoredProofProgram {
            template,
            anchor_x: selected_edge.anchor_x,
            anchor_y: selected_edge.anchor_y,
        }
        .materialize(&source)?;
        if program.cc_cost() != u32::from(selected_edge.cc_cost)
            || program.replay(&source)? != target
        {
            return Err(format!("selected edge {edge_id} failed supervision replay").into());
        }
        replayed_edges += 1;
        let steps = program.semantic_replay_steps(&source)?;
        let cc_index = steps
            .iter()
            .position(|step| step.action.is_crossing_change());
        for (semantic_index, step) in steps.into_iter().enumerate() {
            let Some(task_name) = task(step.action) else {
                excluded_theorem_actions += 1;
                continue;
            };
            let state_encoding = step.state.encode()?;
            let state_sha256 = unknotdb::util::sha256(&state_encoding);
            let action = step.action.encode_u63()?;
            let task_code = u8::from(task_name == "cc_solver");
            let key = (task_code, state_sha256, action);
            let label_id = if let Some(&label_id) = label_ids.get(&key) {
                labels[label_id as usize].occurrences += 1;
                label_id
            } else {
                let label_id: u32 = labels.len().try_into()?;
                labels.push(Label {
                    task: task_name,
                    state_sha256,
                    state_encoding,
                    action,
                    action_kind: action_kind(step.action),
                    strands: step.state.strands,
                    word_length: step.state.word.len().try_into()?,
                    split: split(&state_sha256),
                    occurrences: 1,
                });
                label_ids.insert(key, label_id);
                label_id
            };
            state_actions
                .entry((task_code, state_sha256))
                .or_default()
                .insert(action);
            let phase = match cc_index {
                None => "zero_cc_edge",
                Some(index) if semantic_index < index => "before_cc",
                Some(index) if semantic_index == index => "cc",
                Some(_) => "after_cc",
            };
            occurrences.push(Occurrence {
                label_id,
                source_node,
                edge_id,
                program_id: selected_edge.program_id,
                instruction_index: step.instruction_index,
                phase,
                route_min_cc,
                route_semantic_moves,
            });
        }
    }

    let conflicting_states = state_actions
        .values()
        .filter(|actions| actions.len() > 1)
        .count();
    let preprocessor_labels = labels
        .iter()
        .filter(|label| label.task == "preprocessor")
        .count();
    let cc_labels = labels
        .iter()
        .filter(|label| label.task == "cc_solver")
        .count();
    let train_labels = labels.iter().filter(|label| label.split == "train").count();
    let validation_labels = labels
        .iter()
        .filter(|label| label.split == "validation")
        .count();
    let test_labels = labels.iter().filter(|label| label.split == "test").count();

    let temporary = temporary_path(output)?;
    let mut output_db = Connection::open(&temporary)?;
    output_db.execute_batch(
        r#"
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        PRAGMA user_version=1;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE labels(
            label_id INTEGER PRIMARY KEY,
            task TEXT NOT NULL CHECK(task IN ('preprocessor','cc_solver')),
            state_sha256 BLOB NOT NULL CHECK(length(state_sha256)=32),
            state_encoding BLOB NOT NULL,
            action_u63 INTEGER NOT NULL CHECK(action_u63>=0),
            action_kind TEXT NOT NULL,
            strands INTEGER NOT NULL,
            word_length INTEGER NOT NULL,
            split TEXT NOT NULL CHECK(split IN ('train','validation','test')),
            occurrence_count INTEGER NOT NULL CHECK(occurrence_count>0),
            UNIQUE(task,state_sha256,action_u63)
        );
        CREATE INDEX labels_by_task_split ON labels(task,split,label_id);
        CREATE INDEX labels_by_state ON labels(task,state_sha256);
        CREATE TABLE occurrences(
            occurrence_id INTEGER PRIMARY KEY,
            label_id INTEGER NOT NULL REFERENCES labels(label_id),
            source_node INTEGER NOT NULL,
            edge_id INTEGER NOT NULL,
            program_id INTEGER NOT NULL,
            instruction_index INTEGER NOT NULL,
            phase TEXT NOT NULL CHECK(phase IN ('zero_cc_edge','before_cc','cc','after_cc')),
            route_min_cc INTEGER NOT NULL,
            route_semantic_moves INTEGER NOT NULL
        );
        CREATE INDEX occurrences_by_label ON occurrences(label_id,occurrence_id);
        CREATE INDEX occurrences_by_edge ON occurrences(edge_id,instruction_index);
        CREATE VIEW preprocessor_labels AS
            SELECT * FROM labels WHERE task='preprocessor';
        CREATE VIEW cc_solver_labels AS
            SELECT * FROM labels WHERE task='cc_solver';
        CREATE VIEW preprocessor_stops AS
            SELECT label_id,state_sha256,state_encoding,strands,word_length,split,occurrence_count
            FROM labels WHERE task='cc_solver';
        "#,
    )?;
    let transaction = output_db.transaction()?;
    for (key, value) in [
        ("schema", SCHEMA.to_owned()),
        ("proof_sha256", proof_sha256.clone()),
        ("routing_sha256", routing_sha256.clone()),
        ("route_selection", "shortest_at_min_cc".to_owned()),
        (
            "split_contract",
            "state_sha256[0]: train=0..229 validation=230..242 test=243..255".to_owned(),
        ),
        (
            "exclusions",
            "PASS and theorem/planar collapse actions are not policy labels".to_owned(),
        ),
    ] {
        transaction.execute("INSERT INTO meta VALUES (?1,?2)", params![key, value])?;
    }
    {
        let mut insert =
            transaction.prepare("INSERT INTO labels VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)")?;
        for (label_id, label) in labels.iter().enumerate() {
            insert.execute(params![
                u32::try_from(label_id)?,
                label.task,
                label.state_sha256.as_slice(),
                &label.state_encoding,
                i64::try_from(label.action)?,
                label.action_kind,
                label.strands,
                label.word_length,
                label.split,
                label.occurrences,
            ])?;
        }
    }
    {
        let mut insert =
            transaction.prepare("INSERT INTO occurrences VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)")?;
        for (occurrence_id, occurrence) in occurrences.iter().enumerate() {
            insert.execute(params![
                u32::try_from(occurrence_id)?,
                occurrence.label_id,
                occurrence.source_node,
                occurrence.edge_id,
                occurrence.program_id,
                occurrence.instruction_index,
                occurrence.phase,
                occurrence.route_min_cc,
                i64::try_from(occurrence.route_semantic_moves)?,
            ])?;
        }
    }
    transaction.commit()?;
    output_db.execute_batch("PRAGMA optimize;")?;
    if output_db.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))? != "ok" {
        return Err("supervision dataset integrity check failed".into());
    }
    drop(output_db);
    fs::rename(&temporary, output)?;
    Ok(SupervisionReport {
        selected_edges,
        replayed_edges,
        excluded_planar_edges,
        excluded_theorem_actions,
        preprocessor_labels,
        cc_labels,
        occurrences: occurrences.len(),
        conflicting_states,
        train_labels,
        validation_labels,
        test_labels,
        bytes: fs::metadata(output)?.len(),
        proof_sha256,
        routing_sha256,
    })
}

/// Export conservative set-valued CC supervision from every replay-valid
/// one-CC edge whose target has a finite independently-derived route.
///
/// Options are grouped by the exact physical state immediately before the CC.
/// An option is accepted iff its complete graph cost equals the source node's
/// current minimum CC distance. Higher-cost completed options are comparisons;
/// actions absent from this table remain unknown, never negative examples.
pub fn export_cc_frontier_supervision(
    proof: &Path,
    routing: &Path,
    output: &Path,
) -> Result<CcFrontierReport> {
    if output.exists() {
        return Err(format!("CC frontier dataset already exists: {}", output.display()).into());
    }
    let proof_sha256 = unknotdb::util::sha256_hex(&fs::read(proof)?);
    let routing_sha256 = unknotdb::util::sha256_hex(&fs::read(routing)?);
    let proof_db = Connection::open_with_flags(proof, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let routing_db = Connection::open_with_flags(routing, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let pinned: String = routing_db.query_row(
        "SELECT value FROM meta WHERE key='proof_sha256'",
        [],
        |row| row.get(0),
    )?;
    if pinned != proof_sha256 {
        return Err("routing sidecar is pinned to a different proof snapshot".into());
    }

    let mut route_rows = routing_db.prepare(
        "SELECT node_id,min_cc,shortest_at_min_cc_semantic_moves FROM routes \
         WHERE min_cc IS NOT NULL",
    )?;
    let mut routes = BTreeMap::new();
    for row in route_rows.query_map([], |row| {
        Ok((
            row.get::<_, u32>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })? {
        let (node, cc, moves) = row?;
        routes.insert(node, (cc, u64::try_from(moves)?));
    }

    let mut edge_rows = proof_db.prepare(
        "SELECT e.edge_id,e.source_node,e.target_node,e.program_id, \
                e.program_anchor_x,e.program_anchor_y,p.program_version,p.program, \
                sr.encoding,tr.encoding \
         FROM edges e JOIN programs p USING(program_id) \
         JOIN representations sr ON sr.node_id=e.source_node \
         JOIN representations tr ON tr.node_id=e.target_node \
         WHERE e.cc_cost=1 ORDER BY e.edge_id",
    )?;
    let mut candidate_edges = 0usize;
    let mut replayed_edges = 0usize;
    let mut options = Vec::new();
    let rows = edge_rows.query_map([], |row| {
        Ok((
            row.get::<_, u32>(0)?,
            row.get::<_, u32>(1)?,
            row.get::<_, u32>(2)?,
            row.get::<_, u32>(3)?,
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
            _program_id,
            anchor_x,
            anchor_y,
            version,
            bytes,
            source_bytes,
            target_bytes,
        ) = row?;
        let (Some(&(source_cc, _)), Some(&(target_cc, target_moves))) =
            (routes.get(&source_node), routes.get(&target_node))
        else {
            continue;
        };
        candidate_edges += 1;
        if version != CheckpointedProofProgram::VERSION {
            return Err(format!("unsupported CC frontier program version {version}").into());
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
        if program.cc_cost() != 1 || program.replay(&source)? != target {
            return Err(format!("CC frontier edge {edge_id} failed replay").into());
        }
        replayed_edges += 1;
        let steps = program.semantic_replay_steps(&source)?;
        let semantic_moves = u64::try_from(steps.len())? + target_moves;
        let cc_steps: Vec<_> = steps
            .into_iter()
            .filter(|step| step.action.is_crossing_change())
            .collect();
        if cc_steps.len() != 1 {
            return Err(
                format!("one-CC edge {edge_id} exposes {} CC steps", cc_steps.len()).into(),
            );
        }
        let step = &cc_steps[0];
        let state_encoding = step.state.encode()?;
        let state_sha256 = unknotdb::util::sha256(&state_encoding);
        options.push(CcOption {
            state_sha256,
            state_encoding,
            action: step.action.encode_u63()?,
            strands: step.state.strands,
            word_length: step.state.word.len().try_into()?,
            split: split(&state_sha256),
            source_node,
            edge_id,
            target_node,
            result_cc: target_cc.saturating_add(1),
            result_semantic_moves: semantic_moves,
            accepted: target_cc.saturating_add(1) == source_cc,
        });
    }

    // Exact semantic duplicates can arise from multiple immutable provenance
    // edges. Retain the cheapest deterministic occurrence for training.
    options.sort_by_key(|option| {
        (
            option.state_sha256,
            option.action,
            option.result_cc,
            option.result_semantic_moves,
            option.edge_id,
        )
    });
    options.dedup_by(|left, right| {
        left.state_sha256 == right.state_sha256 && left.action == right.action
    });
    let mut accepted_by_state: BTreeMap<[u8; 32], usize> = BTreeMap::new();
    for option in &options {
        if option.accepted {
            *accepted_by_state.entry(option.state_sha256).or_default() += 1;
        }
    }
    let states = options
        .iter()
        .map(|option| option.state_sha256)
        .collect::<BTreeSet<_>>()
        .len();
    let accepted_options = options.iter().filter(|option| option.accepted).count();
    let multi_accepted_states = accepted_by_state
        .values()
        .filter(|&&count| count > 1)
        .count();

    let temporary = temporary_path(output)?;
    let mut output_db = Connection::open(&temporary)?;
    output_db.execute_batch(
        r#"
        PRAGMA journal_mode=OFF;
        PRAGMA synchronous=OFF;
        PRAGMA user_version=1;
        CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL) WITHOUT ROWID;
        CREATE TABLE options(
            option_id INTEGER PRIMARY KEY,
            state_sha256 BLOB NOT NULL CHECK(length(state_sha256)=32),
            state_encoding BLOB NOT NULL,
            action_u63 INTEGER NOT NULL CHECK(action_u63>=0),
            strands INTEGER NOT NULL,
            word_length INTEGER NOT NULL,
            split TEXT NOT NULL CHECK(split IN ('train','validation','test')),
            source_node INTEGER NOT NULL,
            edge_id INTEGER NOT NULL,
            target_node INTEGER NOT NULL,
            result_cc INTEGER NOT NULL,
            result_semantic_moves INTEGER NOT NULL,
            accepted INTEGER NOT NULL CHECK(accepted IN (0,1)),
            UNIQUE(state_sha256,action_u63)
        );
        CREATE INDEX options_by_split_state ON options(split,state_sha256,action_u63);
        CREATE INDEX options_by_source ON options(source_node,edge_id);
        CREATE VIEW state_frontiers AS
          SELECT state_sha256,split,strands,word_length,
                 count(*) compared_actions,sum(accepted) accepted_actions,
                 min(result_cc) observed_min_cc
          FROM options GROUP BY state_sha256;
        "#,
    )?;
    let transaction = output_db.transaction()?;
    for (key, value) in [
        ("schema", CC_FRONTIER_SCHEMA.to_owned()),
        ("proof_sha256", proof_sha256.clone()),
        ("routing_sha256", routing_sha256.clone()),
        (
            "objective",
            "minimum observed complete CC count; semantic moves retained for audit".to_owned(),
        ),
        (
            "unknown_contract",
            "actions absent from options are unknown and receive zero graph-batch gradient"
                .to_owned(),
        ),
        ("native_internal_action_cap", "5".to_owned()),
    ] {
        transaction.execute("INSERT INTO meta VALUES (?1,?2)", params![key, value])?;
    }
    {
        let mut insert = transaction
            .prepare("INSERT INTO options VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)")?;
        for (option_id, option) in options.iter().enumerate() {
            insert.execute(params![
                u32::try_from(option_id)?,
                option.state_sha256.as_slice(),
                &option.state_encoding,
                i64::try_from(option.action)?,
                option.strands,
                option.word_length,
                option.split,
                option.source_node,
                option.edge_id,
                option.target_node,
                option.result_cc,
                i64::try_from(option.result_semantic_moves)?,
                u8::from(option.accepted),
            ])?;
        }
    }
    transaction.commit()?;
    output_db.execute_batch("PRAGMA optimize;")?;
    if output_db.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))? != "ok" {
        return Err("CC frontier dataset integrity check failed".into());
    }
    drop(output_db);
    fs::rename(&temporary, output)?;
    Ok(CcFrontierReport {
        candidate_edges,
        replayed_edges,
        states,
        options: options.len(),
        accepted_options,
        compared_options: options.len(),
        multi_accepted_states,
        bytes: fs::metadata(output)?.len(),
        proof_sha256,
        routing_sha256,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tasks_exclude_controller_and_theorem_macros() {
        assert_eq!(
            task(SemanticAction::Reduce { position: 0 }),
            Some("preprocessor")
        );
        assert_eq!(
            task(SemanticAction::CrossingChange { position: 0 }),
            Some("cc_solver")
        );
        assert_eq!(task(SemanticAction::Pass), None);
        assert_eq!(task(SemanticAction::DescendingCollapse), None);
        assert_eq!(task(SemanticAction::PlanarCertificateCollapse), None);
    }

    #[test]
    fn state_hash_split_is_disjoint_and_total() {
        let mut counts = [0_usize; 3];
        for first in u8::MIN..=u8::MAX {
            let mut hash = [0_u8; 32];
            hash[0] = first;
            match split(&hash) {
                "train" => counts[0] += 1,
                "validation" => counts[1] += 1,
                "test" => counts[2] += 1,
                other => panic!("unexpected split {other}"),
            }
        }
        assert_eq!(counts, [230, 13, 13]);
    }
}
