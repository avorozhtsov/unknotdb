use crate::descending::attestation;
use crate::policy::{
    compile_preprocessing_instructions, preprocess_to_stopping_point, PolicyLimits, PolicyOracle,
};
use crate::population::{PopulationGraph, RelaxationOutcome};
use crate::representation::{CheckpointedProofProgram, ProofInstruction, SemanticAction};
use crate::{RepKey, Result};

#[derive(Clone, Debug)]
pub struct PlanarU1ImportRun {
    pub source_key: RepKey,
    pub successor_key: RepKey,
    pub certificate_id: RepKey,
    pub old_u: u32,
    pub new_u: u32,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub planar_outcome: RelaxationOutcome,
    pub crossing_outcome: RelaxationOutcome,
    pub manifest: String,
}

#[derive(Clone, Debug)]
pub struct PlanarCc0ImportRun {
    pub source_key: RepKey,
    pub certificate_id: RepKey,
    pub old_u: u32,
    pub new_u: u32,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub outcome: RelaxationOutcome,
    pub manifest: String,
}

/// Attach an exact planar RI/RII/RIII unknot certificate directly to an
/// existing normalized stopping point.  The certificate collapse has CC cost
/// zero; the independent edge verifier replays the complete labelled trace
/// before the edge is admitted.
pub fn import_planar_cc0(
    graph: &mut PopulationGraph,
    source_key: RepKey,
    certificate: Vec<u8>,
    source_snapshot_sha256: &str,
) -> Result<PlanarCc0ImportRun> {
    let before_nodes = graph.node_count();
    let before_edges = graph.edge_count();
    let source = graph
        .expansion_seed(&source_key)?
        .ok_or("planar CC0 import source key is absent")?;
    let old_u = source.u_upper_bound;
    crate::planar::verify_certificate(&source.representation.representation, &certificate)?;
    let certificate_id = unknotdb::util::sha256(&certificate);
    let unknot_key = graph.unknot_key()?;
    let outcome = graph.relax_planar_certificate_edge(
        source.representation,
        source.policy_stop,
        unknot_key,
        certificate,
    )?;
    let new_u = graph
        .u_upper_bound(&source_key)
        .ok_or("planar CC0 import source disappeared")?;
    if new_u != 0 {
        return Err(format!("planar CC0 import produced source bound {new_u}").into());
    }
    let inserted_nodes = graph.node_count() - before_nodes;
    let inserted_edges = graph.edge_count() - before_edges;
    let manifest = format!(
        "format\tunknotdb-planar-cc0-import-v0\n\
         source_snapshot_sha256\t{source_snapshot_sha256}\n\
         source_key\t{}\n\
         certificate_id\t{}\n\
         old_u\t{old_u}\n\
         new_u\t{new_u}\n\
         inserted_nodes\t{inserted_nodes}\n\
         inserted_edges\t{inserted_edges}\n\
         outcome\t{outcome:?}\n",
        hex(&source_key),
        hex(&certificate_id),
    );
    Ok(PlanarCc0ImportRun {
        source_key,
        certificate_id,
        old_u,
        new_u,
        inserted_nodes,
        inserted_edges,
        outcome,
        manifest,
    })
}

pub fn import_planar_u1<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    source_key: RepKey,
    crossing_position: u32,
    certificate: Vec<u8>,
    oracle: &mut O,
    policy_limits: PolicyLimits,
    source_snapshot_sha256: &str,
) -> Result<PlanarU1ImportRun> {
    let before_nodes = graph.node_count();
    let before_edges = graph.edge_count();
    let source = graph
        .expansion_seed(&source_key)?
        .ok_or("planar import source key is absent")?;
    let old_u = source.u_upper_bound;
    let crossing_change = SemanticAction::CrossingChange {
        position: crossing_position,
    };
    let raw_successor = crossing_change.apply(&source.representation.representation)?;
    let preprocessing = preprocess_to_stopping_point(&raw_successor, oracle, policy_limits)?;
    if !preprocessing.is_graph_stopping_point() {
        return Err(format!(
            "post-CC preprocessing stopped at {:?}",
            preprocessing.stop_reason
        )
        .into());
    }
    crate::planar::verify_certificate(&preprocessing.output.representation, &certificate)?;
    let successor_stop = attestation(&preprocessing)?;
    let successor_key = preprocessing.output.key;
    let certificate_id = unknotdb::util::sha256(&certificate);
    let unknot_key = graph.unknot_key()?;
    let planar_outcome = graph.relax_planar_certificate_edge(
        preprocessing.output.clone(),
        successor_stop,
        unknot_key,
        certificate,
    )?;

    let mut instructions = vec![ProofInstruction::Action(crossing_change)];
    instructions.extend(compile_preprocessing_instructions(&preprocessing)?);
    let crossing_program = CheckpointedProofProgram { instructions }
        .canonicalize_semantic_first(&source.representation.representation)?;
    let crossing_outcome = graph.relax_unknot_edge(
        source.representation,
        source.policy_stop,
        successor_key,
        crossing_program,
        None,
    )?;
    let new_u = graph
        .u_upper_bound(&source_key)
        .ok_or("planar import source disappeared")?;
    if new_u != 1 {
        return Err(format!("planar U=1 import produced source bound {new_u}").into());
    }
    let inserted_nodes = graph.node_count() - before_nodes;
    let inserted_edges = graph.edge_count() - before_edges;
    let manifest = format!(
        "format\tunknotdb-planar-u1-import-v0\n\
         source_snapshot_sha256\t{source_snapshot_sha256}\n\
         source_key\t{}\n\
         crossing_position\t{crossing_position}\n\
         successor_key\t{}\n\
         certificate_id\t{}\n\
         old_u\t{old_u}\n\
         new_u\t{new_u}\n\
         inserted_nodes\t{inserted_nodes}\n\
         inserted_edges\t{inserted_edges}\n\
         planar_outcome\t{planar_outcome:?}\n\
         crossing_outcome\t{crossing_outcome:?}\n",
        hex(&source_key),
        hex(&successor_key),
        hex(&certificate_id),
    );
    Ok(PlanarU1ImportRun {
        source_key,
        successor_key,
        certificate_id,
        old_u,
        new_u,
        inserted_nodes,
        inserted_edges,
        planar_outcome,
        crossing_outcome,
        manifest,
    })
}

fn hex(bytes: &RepKey) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
