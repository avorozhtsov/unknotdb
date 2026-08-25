//! Fixed-budget graph improvement, reverse expansion and regression reports.

use crate::census::enumerate_natural_braids;
use crate::frontier::{
    enumerate_scrambles_detailed, expand_frontier_seed, FrontierBatch, FrontierDisposition,
    ScrambleLimits,
};
use crate::policy::{preprocess_many_to_stopping_points, PolicyLimits, PolicyOracle};
use crate::population::{ExpansionSeed, PopulationGraph, RelaxationOutcome};
use crate::representation::{BraidRepresentation, CheckpointedProofProgram, ProofInstruction};
use crate::{GraphSnapshot, Result, SqliteSnapshot};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

pub const B4_REGRESSION_VERSION: &str = "unknotdb-b4-regression-v0";
pub const QUALITY_MANIFEST_VERSION: &str = "unknotdb-quality-frontier-v0";
pub const REVERSE_MANIFEST_VERSION: &str = "unknotdb-bidirectional-frontier-v0";
pub const B5_BOUNDARY_VERSION: &str = "unknotdb-selective-b5-boundary-v0";
pub const ACS10_REPORT_VERSION: &str = "unknotdb-action-choice-score-10-report-v0";

pub fn verify_key_superset(baseline_path: &Path, candidate_path: &Path) -> Result<usize> {
    let baseline = GraphSnapshot::load_file(baseline_path)?;
    let candidate = GraphSnapshot::load_file(candidate_path)?;
    for id in 0..baseline.node_count() {
        let key = baseline
            .key_for_node(id.try_into().map_err(|_| "baseline node id exceeds u32")?)
            .ok_or("baseline key arena is incomplete")?;
        if candidate.lookup(key).is_none() {
            return Err(format!("candidate snapshot lost baseline key {}", hex(key)).into());
        }
    }
    Ok(baseline.node_count())
}

#[derive(Clone, Debug)]
pub struct B4RegressionReport {
    pub snapshot_sha256: String,
    pub seed_manifest_sha256: String,
    pub nodes: usize,
    pub edges: usize,
    pub covered_seeds: usize,
    pub total_seeds: usize,
    pub direct_key_hits: usize,
    pub replayed_edges: usize,
    pub u_p50: u32,
    pub u_p95: u32,
    pub u_max: u32,
    pub zero_cc_p50: u32,
    pub zero_cc_p95: u32,
    pub zero_cc_max: u32,
    pub zero_cc_edges: usize,
    pub snapshot_bytes: u64,
    pub coverage_time: Duration,
    pub replay_time: Duration,
    pub lookup_time: Duration,
    pub lookup_queries: usize,
}

impl B4RegressionReport {
    pub fn run(snapshot_path: &Path, seed_manifest_path: &Path) -> Result<Self> {
        let snapshot_sha256 = file_sha256(snapshot_path)?;
        let seed_manifest_sha256 = file_sha256(seed_manifest_path)?;
        let seed_manifest = fs::read_to_string(seed_manifest_path)?;
        let (covered_seeds, total_seeds) = parse_completion_coverage(&seed_manifest)?;
        let cold = SqliteSnapshot::open_file(snapshot_path)?;
        cold.verify_program_hashes()?;
        let replay_started = Instant::now();
        let replayed_edges = cold.verify_full_replay()?;
        let replay_time = replay_started.elapsed();
        let graph = GraphSnapshot::load_connection(cold.connection())?;

        let coverage_started = Instant::now();
        let enumeration = enumerate_natural_braids(4, 10)?;
        let mut direct_keys = Vec::new();
        for seed in &enumeration.seeds {
            if graph.lookup(&seed.representation.key).is_some() {
                direct_keys.push(seed.representation.key);
            }
        }
        let coverage_time = coverage_started.elapsed();
        if total_seeds != enumeration.seeds.len() || covered_seeds != total_seeds {
            return Err(format!(
                "B4 completion manifest coverage is {covered_seeds}/{total_seeds}, enumerator has {} seeds",
                enumeration.seeds.len()
            )
            .into());
        }

        let mut u_values = Vec::with_capacity(graph.node_count());
        let mut stmt = cold
            .connection()
            .prepare("SELECT u_upper_bound FROM nodes ORDER BY u_upper_bound")?;
        for row in stmt.query_map([], |row| row.get::<_, u32>(0))? {
            u_values.push(row?);
        }
        let mut zero_cc_lengths = Vec::new();
        let mut stmt = cold.connection().prepare(
            "SELECT p.program FROM edges e JOIN programs p ON p.program_id=e.program_id \
             WHERE e.cc_cost=0",
        )?;
        for row in stmt.query_map([], |row| row.get::<_, Vec<u8>>(0))? {
            let program = CheckpointedProofProgram::decode(&row?)?;
            zero_cc_lengths.push(semantic_len(&program)?);
        }
        zero_cc_lengths.sort_unstable();

        let lookup_queries = 100_000_usize;
        let lookup_started = Instant::now();
        let mut checksum = 0_u64;
        for index in 0..lookup_queries {
            let key = &direct_keys[index % direct_keys.len()];
            let hit = graph
                .lookup(key)
                .ok_or("covered B4 key disappeared during lookup benchmark")?;
            checksum = checksum.wrapping_add(u64::from(hit.node_id));
        }
        std::hint::black_box(checksum);
        let lookup_time = lookup_started.elapsed();

        Ok(Self {
            snapshot_sha256,
            seed_manifest_sha256,
            nodes: graph.node_count(),
            edges: graph.edge_count(),
            covered_seeds,
            total_seeds,
            direct_key_hits: direct_keys.len(),
            replayed_edges,
            u_p50: percentile(&u_values, 50),
            u_p95: percentile(&u_values, 95),
            u_max: *u_values.last().unwrap_or(&0),
            zero_cc_p50: percentile(&zero_cc_lengths, 50),
            zero_cc_p95: percentile(&zero_cc_lengths, 95),
            zero_cc_max: *zero_cc_lengths.last().unwrap_or(&0),
            zero_cc_edges: zero_cc_lengths.len(),
            snapshot_bytes: fs::metadata(snapshot_path)?.len(),
            coverage_time,
            replay_time,
            lookup_time,
            lookup_queries,
        })
    }

    pub fn gate_text(&self) -> String {
        format!(
            "{B4_REGRESSION_VERSION}\n\
snapshot_sha256={}\nseed_manifest_sha256={}\n\
nodes={}\nedges={}\npreprocessed_coverage={}/{}\ndirect_key_hits={}/{}\nreplayed_edges={}\n\
u_p50={}\nu_p95={}\nu_max={}\n\
zero_cc_edges={}\nzero_cc_semantic_len_p50={}\nzero_cc_semantic_len_p95={}\nzero_cc_semantic_len_max={}\n\
snapshot_bytes={}\nbytes_per_node={:.3}\nbytes_per_edge={:.3}\n\
coverage_seconds={:.6}\nreplay_seconds={:.6}\n\
lookup_queries={}\nlookup_seconds={:.6}\nlookup_per_second={:.0}\n",
            self.snapshot_sha256,
            self.seed_manifest_sha256,
            self.nodes,
            self.edges,
            self.covered_seeds,
            self.total_seeds,
            self.direct_key_hits,
            self.total_seeds,
            self.replayed_edges,
            self.u_p50,
            self.u_p95,
            self.u_max,
            self.zero_cc_edges,
            self.zero_cc_p50,
            self.zero_cc_p95,
            self.zero_cc_max,
            self.snapshot_bytes,
            self.snapshot_bytes as f64 / self.nodes as f64,
            self.snapshot_bytes as f64 / self.edges as f64,
            self.coverage_time.as_secs_f64(),
            self.replay_time.as_secs_f64(),
            self.lookup_queries,
            self.lookup_time.as_secs_f64(),
            self.lookup_queries as f64 / self.lookup_time.as_secs_f64(),
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub struct QualityLimits {
    pub seed_limit: usize,
    pub max_depth: u16,
    pub max_states_per_depth: u32,
    pub max_candidates_per_depth: u32,
    pub max_strands: u16,
    pub max_word_length: u32,
}

#[derive(Clone, Debug)]
pub struct QualityRun {
    pub limits: QualityLimits,
    pub attempted_seeds: usize,
    pub enumerated_candidates: usize,
    pub known_target_candidates: usize,
    pub accepted_edges: usize,
    pub improved_nodes: usize,
    pub total_u_decrease: u64,
    pub manifest: String,
}

pub fn improve_quality(graph: &mut PopulationGraph, limits: QualityLimits) -> Result<QualityRun> {
    if limits.seed_limit == 0 || limits.max_depth == 0 {
        return Err("quality seed/depth limits must be positive".into());
    }
    let seeds = graph.ranked_quality_seeds(limits.seed_limit)?;
    let mut manifest = format!(
        "{QUALITY_MANIFEST_VERSION}\nlimits\t{}\t{}\t{}\t{}\t{}\t{}\n",
        limits.seed_limit,
        limits.max_depth,
        limits.max_states_per_depth,
        limits.max_candidates_per_depth,
        limits.max_strands,
        limits.max_word_length
    );
    manifest
        .push_str("accepted\tsource_key\ttarget_key\told_u\tnew_u\tdepth\tsemantic_len\tprogram\n");
    let mut enumerated_candidates = 0;
    let mut known_target_candidates = 0;
    let mut accepted_edges = 0;
    let mut improved_nodes = BTreeSet::new();
    let mut total_u_decrease = 0_u64;
    for seed in &seeds {
        for depth in 1..=limits.max_depth {
            let enumeration = enumerate_scrambles_detailed(
                &seed.representation.representation,
                ScrambleLimits {
                    depth,
                    max_states: limits.max_states_per_depth,
                    max_candidates: limits.max_candidates_per_depth,
                    max_strands: limits.max_strands,
                    max_word_length: limits.max_word_length,
                    min_cc_cost: 1,
                    max_cc_cost: 1,
                },
            )?;
            enumerated_candidates += enumeration.candidates.len();
            for candidate in enumeration.candidates {
                let target = candidate.output.normalize()?;
                let Some(target_u) = graph.u_upper_bound(&target.key) else {
                    continue;
                };
                known_target_candidates += 1;
                if target.key == seed.representation.key
                    || target_u.saturating_add(1) >= seed.u_upper_bound
                {
                    continue;
                }
                let mut instructions: Vec<_> = candidate
                    .actions
                    .iter()
                    .copied()
                    .map(ProofInstruction::Action)
                    .collect();
                push_normalization(&mut instructions, target.witness);
                let program = CheckpointedProofProgram { instructions }
                    .canonicalize_semantic_first(&seed.representation.representation)?;
                if program.replay(&seed.representation.representation)? != target.representation {
                    return Err("quality candidate failed exact normalized replay".into());
                }
                let encoded = program.encode()?;
                match graph.relax_unknot_edge_deferred_acs10(
                    seed.representation.clone(),
                    seed.policy_stop,
                    target.key,
                    program,
                    None,
                )? {
                    RelaxationOutcome::Improved { old, new } => {
                        accepted_edges += 1;
                        improved_nodes.insert(seed.representation.key);
                        total_u_decrease += u64::from(old - new);
                        manifest.push_str(&format!(
                            "accepted\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                            hex(&seed.representation.key),
                            hex(&target.key),
                            old,
                            new,
                            depth,
                            candidate.actions.len(),
                            hex(&encoded)
                        ));
                    }
                    RelaxationOutcome::AcceptedAcs10 { .. }
                    | RelaxationOutcome::Unchanged { .. }
                    | RelaxationOutcome::Inserted { .. } => {}
                }
            }
        }
    }
    graph.recompute_acs10_routes()?;
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\t{}\n",
        seeds.len(),
        enumerated_candidates,
        known_target_candidates,
        accepted_edges,
        total_u_decrease
    ));
    Ok(QualityRun {
        limits,
        attempted_seeds: seeds.len(),
        enumerated_candidates,
        known_target_candidates,
        accepted_edges,
        improved_nodes: improved_nodes.len(),
        total_u_decrease,
        manifest,
    })
}

#[derive(Clone, Copy, Debug)]
pub struct ReverseLimits {
    pub seed_limit: usize,
    pub min_seed_strands: u16,
    pub policy_limits: PolicyLimits,
    pub scramble_limits: ScrambleLimits,
}

#[derive(Clone, Debug)]
pub struct ReverseRun {
    pub batches: Vec<(ExpansionSeed, FrontierBatch)>,
    pub parent_nodes: usize,
    pub parent_edges: usize,
    pub final_nodes: usize,
    pub final_edges: usize,
    pub inserted: usize,
    pub improved: usize,
    pub accepted_acs10: usize,
    pub manifest: String,
}

pub fn reverse_expand<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    oracle: &mut O,
    limits: ReverseLimits,
) -> Result<ReverseRun> {
    let parent_nodes = graph.node_count();
    let parent_edges = graph.edge_count();
    let seeds = graph
        .ranked_short_state_seeds_with_min_strands(limits.seed_limit, limits.min_seed_strands)?;
    let mut batches = Vec::new();
    let mut inserted = 0;
    let mut improved = 0;
    let mut accepted_acs10 = 0;
    let mut manifest = format!(
        "{REVERSE_MANIFEST_VERSION}\nmodel_id\t{}\nlimits\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        oracle.model_id(),
        limits.seed_limit,
        limits.policy_limits.max_policy_plies,
        limits.policy_limits.max_semantic_moves,
        limits.scramble_limits.depth,
        limits.scramble_limits.max_states,
        limits.scramble_limits.max_candidates,
        limits.scramble_limits.max_strands,
        limits.scramble_limits.max_word_length,
        limits.scramble_limits.max_cc_cost,
    );
    for seed in seeds {
        let batch = expand_frontier_seed(
            graph,
            &seed.representation,
            oracle,
            limits.policy_limits,
            limits.scramble_limits,
        )?;
        for attempt in &batch.attempts {
            match attempt.disposition {
                FrontierDisposition::Inserted { .. } => inserted += 1,
                FrontierDisposition::Improved { .. } => improved += 1,
                FrontierDisposition::AcceptedAcs10 { .. } => accepted_acs10 += 1,
                _ => {}
            }
        }
        manifest.push_str(&format!(
            "seed\t{}\t{}\t{}\t{}\n",
            hex(&seed.representation.key),
            seed.u_upper_bound,
            seed.representation.representation.strands,
            seed.representation.representation.word.len()
        ));
        manifest.push_str(&batch.manifest_tsv());
        batches.push((seed, batch));
    }
    let final_nodes = graph.node_count();
    let final_edges = graph.edge_count();
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        parent_nodes, parent_edges, final_nodes, final_edges, inserted, improved, accepted_acs10
    ));
    Ok(ReverseRun {
        batches,
        parent_nodes,
        parent_edges,
        final_nodes,
        final_edges,
        inserted,
        improved,
        accepted_acs10,
        manifest,
    })
}

#[derive(Clone, Debug)]
pub struct B5BoundaryReport {
    pub cohort_size: usize,
    pub distinct_stopping_points: usize,
    pub graph_hits: usize,
    pub incomplete: usize,
    pub manifest: String,
}

pub fn measure_b5_cohort<O: PolicyOracle>(
    graph: &PopulationGraph,
    oracle: &mut O,
    policy_limits: PolicyLimits,
    cohort_size: usize,
) -> Result<B5BoundaryReport> {
    let cohort = representative_b5_cohort(cohort_size)?;
    let inputs: Vec<_> = cohort
        .iter()
        .map(|representation| representation.representation.clone())
        .collect();
    let reports = preprocess_many_to_stopping_points(&inputs, oracle, policy_limits)?;
    let mut distinct = BTreeSet::new();
    let mut hits = 0;
    let mut incomplete = 0;
    let mut manifest = format!(
        "{B5_BOUNDARY_VERSION}\nmodel_id\t{}\ncohort\t{}\t5\t4\t10\tdeterministic-lcg-natural-words-v0\n",
        oracle.model_id(), cohort_size
    );
    manifest.push_str("seed\tseed_key\tword_length\tstop_reason\tstopping_key\thit\n");
    for (seed, report) in cohort.iter().zip(reports) {
        let stopping_key = report
            .is_graph_stopping_point()
            .then_some(report.output.key);
        if let Some(key) = stopping_key {
            distinct.insert(key);
            hits += usize::from(graph.u_upper_bound(&key).is_some());
        } else {
            incomplete += 1;
        }
        manifest.push_str(&format!(
            "seed\t{}\t{}\t{:?}\t{}\t{}\n",
            hex(&seed.key),
            seed.representation.word.len(),
            report.stop_reason,
            stopping_key
                .as_ref()
                .map(|key| hex(key))
                .unwrap_or_else(|| "-".into()),
            u8::from(stopping_key.is_some_and(|key| graph.u_upper_bound(&key).is_some()))
        ));
    }
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\n",
        cohort.len(),
        distinct.len(),
        hits,
        incomplete
    ));
    Ok(B5BoundaryReport {
        cohort_size: cohort.len(),
        distinct_stopping_points: distinct.len(),
        graph_hits: hits,
        incomplete,
        manifest,
    })
}

pub fn acs10_comparison_report(snapshot_path: &Path, sample_limit: usize) -> Result<String> {
    let (graph, meta) = PopulationGraph::from_snapshot(snapshot_path)?;
    let (nodes, edges) = graph.into_snapshot_records()?;
    let mut different = 0_usize;
    let mut both = 0_usize;
    let mut acs_only = 0_usize;
    let mut u_only = 0_usize;
    let mut samples = Vec::new();
    for node in &nodes {
        match (node.next_unknot, node.next_acs10) {
            (Some(u), Some(a)) => {
                both += 1;
                if u.edge_id != a.edge_id {
                    different += 1;
                    if samples.len() < sample_limit {
                        let u_edge = &edges[u.edge_id as usize];
                        let a_edge = &edges[a.edge_id as usize];
                        samples.push(format!(
                            "different\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                            hex(&node.rep_key),
                            node.u_upper_bound.unwrap_or(0),
                            node.acs10.unwrap_or(0),
                            hex(&nodes[u_edge.target_node as usize].rep_key),
                            hex(&nodes[a_edge.target_node as usize].rep_key),
                            u_edge.cc_cost,
                            a_edge.cc_cost,
                        ));
                    }
                }
            }
            (None, Some(_)) => acs_only += 1,
            (Some(_), None) => u_only += 1,
            (None, None) => {}
        }
    }
    let mut text = format!(
        "{ACS10_REPORT_VERSION}\n\
snapshot_source_generation\t{}\n\
definition\tACS10(v)=10*strands(v)+5*U(v)+word_length(v)\n\
semantics\tAction-Choice Score 10\n\
eligibility\tACS10(target)<ACS10(source)+cc(edge) and target_rank<source_rank\n\
selection\tmin(ACS10(target)+cc,ACS10(target),target_rank,target_key,edge_id)\n\
independence\tnext_acs10 is recomputed from all verified immutable edges independently of next_unknot\n\
nodes\t{}\nedges\t{}\n\
routes\t{}\t{}\t{}\t{}\n\
sample_header\tsource_key\tu_upper\tacs10\tu_target\tacs10_target\tu_edge_cc\tacs10_edge_cc\n",
        meta.source_generation,
        nodes.len(),
        edges.len(),
        both,
        different,
        acs_only,
        u_only,
    );
    for sample in samples {
        text.push_str(&sample);
    }
    Ok(text)
}

fn representative_b5_cohort(
    limit: usize,
) -> Result<Vec<crate::representation::NormalizedRepresentation>> {
    if limit == 0 {
        return Err("B5 cohort size must be positive".into());
    }
    let mut by_key = BTreeMap::new();
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut attempts = 0_usize;
    while by_key.len() < limit && attempts < limit.saturating_mul(1000) {
        attempts += 1;
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let length = 4 + (state as usize % 7);
        let mut code = state.rotate_left(17);
        let mut word = Vec::with_capacity(length);
        for _ in 0..length {
            let digit = code & 7;
            code = code.rotate_right(7) ^ code.wrapping_mul(0x94d0_49bb_1331_11eb);
            let generator = (digit / 2 + 1) as i16;
            word.push(if digit.is_multiple_of(2) {
                generator
            } else {
                -generator
            });
        }
        let representation = BraidRepresentation {
            strands: 5,
            cyclic_band_generators: false,
            word,
        };
        if representation.is_knot_closure()? {
            let normalized = representation.normalize()?;
            by_key.entry(normalized.key).or_insert(normalized);
        }
    }
    if by_key.len() != limit {
        return Err(format!(
            "could only generate {} distinct B5 cohort seeds",
            by_key.len()
        )
        .into());
    }
    Ok(by_key.into_values().collect())
}

fn push_normalization(
    instructions: &mut Vec<ProofInstruction>,
    witness: crate::representation::NormalizationWitness,
) {
    if witness.mirrored {
        instructions.push(ProofInstruction::MirrorOrbit);
    }
    if witness.rotate_word_left != 0 {
        instructions.push(ProofInstruction::NormalizeOrigin {
            rotate_word_left: witness.rotate_word_left,
        });
    }
}

fn semantic_len(program: &CheckpointedProofProgram) -> Result<u32> {
    program
        .instructions
        .iter()
        .filter(|instruction| matches!(instruction, ProofInstruction::Action(_)))
        .count()
        .try_into()
        .map_err(|_| "semantic program length exceeds u32".into())
}

fn percentile(sorted: &[u32], percentile: usize) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() - 1) * percentile).div_ceil(100);
    sorted[index]
}

fn parse_completion_coverage(manifest: &str) -> Result<(usize, usize)> {
    let line = manifest
        .lines()
        .find(|line| line.starts_with("coverage\t"))
        .ok_or("completion manifest has no coverage row")?;
    let fields: Vec<_> = line.split('\t').collect();
    if fields.len() != 7 || fields[1] != "4" || fields[2] != "10" {
        return Err("completion manifest is not the frozen B4 length-10 corpus".into());
    }
    Ok((fields[6].parse()?, fields[3].parse()?))
}

fn file_sha256(path: &Path) -> Result<String> {
    Ok(unknotdb::util::sha256_hex(&fs::read(path)?))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn representative_b5_cohort_is_fixed_and_knotted() {
        let first = representative_b5_cohort(16).unwrap();
        let second = representative_b5_cohort(16).unwrap();
        assert_eq!(first, second);
        assert!(first.iter().all(|seed| seed.representation.strands == 5));
        assert!(first
            .iter()
            .all(|seed| seed.representation.is_knot_closure().unwrap()));
    }

    #[test]
    fn percentile_uses_nearest_rank_ceiling() {
        assert_eq!(percentile(&[1, 2, 3, 4], 50), 3);
        assert_eq!(percentile(&[1, 2, 3, 4], 95), 4);
        assert_eq!(percentile(&[], 95), 0);
    }
}
