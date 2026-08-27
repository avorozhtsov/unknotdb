//! Manifest-backed, query-driven population from the read-only RF corpus.

use crate::optimizer::{
    optimize_vertex, OptimizerBudget, OptimizerPolicyProfile, OptimizerProvenance,
};
use crate::policy::{preprocess_many_to_stopping_points, PolicyLimits, PolicyOracle};
use crate::population::PopulationGraph;
use crate::representation::BraidRepresentation;
use crate::targeted::{force_connected_insert, ConnectedInsertDisposition, ConnectedInsertLimits};
use crate::{RepKey, Result};
use std::fs;
use std::path::Path;

pub const RF_CAMPAIGN_MANIFEST_VERSION: &str = "unknotdb-rf-targeted-campaign-v0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RfCorpusEntry {
    pub representation_id: String,
    pub priority: u8,
    pub representation: BraidRepresentation,
    pub roles: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RfCampaignLimits {
    pub max_priority: u8,
    pub max_input_strands: u16,
    pub max_input_word_length: u32,
    pub max_insert_attempts: usize,
    pub skip_missing: usize,
    pub total_insert_simulations: u32,
    pub per_item_simulations: u32,
    pub max_track_depth: u16,
    pub total_optimizer_simulations: u32,
    pub per_item_optimizer_simulations: u32,
    pub optimize_above_u: u32,
    pub policy_limits: PolicyLimits,
}

impl RfCampaignLimits {
    fn validate(self) -> Result<()> {
        if self.max_insert_attempts == 0
            || self.total_insert_simulations == 0
            || self.per_item_simulations == 0
            || self.per_item_optimizer_simulations == 0
        {
            return Err("RF campaign item and simulation budgets must be positive".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RfCampaignRun {
    pub corpus_rows: usize,
    pub selected_rows: usize,
    pub capacity_excluded: usize,
    pub complete_preprocessing: usize,
    pub initially_covered: usize,
    pub initially_missing: usize,
    pub insert_attempts: usize,
    pub inserted_or_connected: usize,
    pub remaining_misses: usize,
    pub insert_simulations: u32,
    pub policy_only_found: usize,
    pub graph_assisted_found: usize,
    pub optimizer_attempts: usize,
    pub optimizer_improvements: usize,
    pub optimizer_simulations: u32,
    pub inserted_nodes: usize,
    pub inserted_edges: usize,
    pub total_u_decrease: u64,
    pub manifest: String,
}

pub fn read_rf_corpus_tsv(path: &Path) -> Result<Vec<RfCorpusEntry>> {
    let text = fs::read_to_string(path)?;
    let mut lines = text.lines();
    if lines.next() != Some("representation_id\tpriority\tstrands\tword\troles") {
        return Err("RF corpus TSV has an unsupported header".into());
    }
    let mut rows = Vec::new();
    for (line_index, line) in lines.enumerate() {
        let fields: Vec<_> = line.split('\t').collect();
        if fields.len() != 5 {
            return Err(format!(
                "RF corpus line {} has {} fields",
                line_index + 2,
                fields.len()
            )
            .into());
        }
        let word = if fields[3].is_empty() {
            Vec::new()
        } else {
            fields[3]
                .split(',')
                .map(str::parse::<i16>)
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let representation = BraidRepresentation {
            strands: fields[2].parse()?,
            cyclic_band_generators: false,
            word,
        };
        representation.validate()?;
        if !representation.is_knot_closure()? {
            return Err(format!("RF corpus line {} is not a knot closure", line_index + 2).into());
        }
        rows.push(RfCorpusEntry {
            representation_id: fields[0].to_owned(),
            priority: fields[1].parse()?,
            representation,
            roles: fields[4].to_owned(),
        });
    }
    Ok(rows)
}

/// Audit every selected representation, then spend fixed aggregate budgets on
/// broad miss coverage and strict-U improvements. The caller publishes only
/// the final graph, so an interrupted or failed run cannot expose a partial DB.
pub fn run_rf_campaign<O: PolicyOracle>(
    graph: &mut PopulationGraph,
    corpus: &[RfCorpusEntry],
    oracle: &mut O,
    snapshot_id: &str,
    corpus_id: &str,
    limits: RfCampaignLimits,
) -> Result<RfCampaignRun> {
    limits.validate()?;
    if snapshot_id.is_empty() || corpus_id.is_empty() {
        return Err("RF campaign snapshot/corpus IDs must be non-empty".into());
    }
    let priority_selected: Vec<_> = corpus
        .iter()
        .filter(|entry| entry.priority <= limits.max_priority)
        .collect();
    let selected: Vec<_> = priority_selected
        .iter()
        .copied()
        .filter(|entry| {
            entry.representation.strands <= limits.max_input_strands
                && entry.representation.word.len() <= limits.max_input_word_length as usize
        })
        .collect();
    let capacity_excluded = priority_selected.len() - selected.len();
    let inputs: Vec<_> = selected
        .iter()
        .map(|entry| entry.representation.clone())
        .collect();
    let reports = preprocess_many_to_stopping_points(&inputs, oracle, limits.policy_limits)?;
    let mut complete_preprocessing = 0_usize;
    let mut initially_covered = 0_usize;
    let mut missing = Vec::new();
    let mut optimization_candidates = Vec::<(RepKey, u32, String)>::new();
    let mut manifest = format!(
        "{RF_CAMPAIGN_MANIFEST_VERSION}\nsnapshot_id\t{snapshot_id}\ncorpus_id\t{corpus_id}\nmodel_id\t{}\nlimits\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        oracle.model_id(),
        limits.max_priority,
        limits.max_input_strands,
        limits.max_input_word_length,
        limits.max_insert_attempts,
        limits.skip_missing,
        limits.total_insert_simulations,
        limits.per_item_simulations,
        limits.max_track_depth,
        limits.total_optimizer_simulations,
        limits.per_item_optimizer_simulations,
        limits.optimize_above_u,
        limits.policy_limits.max_policy_plies,
        limits.policy_limits.max_semantic_moves,
    );
    manifest.push_str("audit\trepresentation_id\tpriority\tstop_reason\tstopping_key\thit\tu\n");
    for ((entry, report), ordinal) in selected.iter().zip(&reports).zip(0_usize..) {
        if !report.is_graph_stopping_point() {
            manifest.push_str(&format!(
                "audit\t{}\t{}\t{:?}\t-\t0\t-\n",
                entry.representation_id, entry.priority, report.stop_reason
            ));
            continue;
        }
        complete_preprocessing += 1;
        let u = graph.u_upper_bound(&report.output.key);
        if let Some(value) = u {
            initially_covered += 1;
            if value > limits.optimize_above_u {
                optimization_candidates.push((
                    report.output.key,
                    value,
                    entry.representation_id.clone(),
                ));
            }
        } else {
            missing.push(ordinal);
        }
        manifest.push_str(&format!(
            "audit\t{}\t{}\t{:?}\t{}\t{}\t{}\n",
            entry.representation_id,
            entry.priority,
            report.stop_reason,
            hex(&report.output.key),
            u8::from(u.is_some()),
            optional_u32(u),
        ));
    }

    let parent_nodes = graph.node_count();
    let parent_edges = graph.edge_count();
    let before_u = graph.u_bounds();
    let initially_missing = missing.len();
    let mut insert_attempts = 0_usize;
    let mut inserted_or_connected = 0_usize;
    let mut remaining_misses = initially_missing;
    let mut insert_simulations = 0_u32;
    let mut policy_only_found = 0_usize;
    let mut graph_assisted_found = 0_usize;
    for ordinal in missing
        .into_iter()
        .skip(limits.skip_missing)
        .take(limits.max_insert_attempts)
    {
        let remaining = limits
            .total_insert_simulations
            .saturating_sub(insert_simulations);
        if remaining == 0 {
            break;
        }
        let item_budget = remaining.min(limits.per_item_simulations);
        let entry = selected[ordinal];
        let result = force_connected_insert(
            graph,
            &entry.representation,
            oracle,
            ConnectedInsertLimits {
                max_simulations: item_budget,
                max_search_states: item_budget,
                max_track_depth: limits.max_track_depth,
                policy_limits: limits.policy_limits,
            },
            &entry.representation_id,
            snapshot_id,
        )?;
        insert_attempts += 1;
        insert_simulations = insert_simulations
            .checked_add(result.simulations)
            .ok_or("RF campaign insertion simulation overflow")?;
        policy_only_found += usize::from(result.comparison.policy_only_found);
        graph_assisted_found += usize::from(result.comparison.graph_assisted_found);
        manifest.push_str(&format!("connected_begin\t{}\n", entry.representation_id));
        manifest.push_str(&result.manifest);
        manifest.push_str(&format!("connected_end\t{}\n", entry.representation_id));
        if matches!(
            result.disposition,
            ConnectedInsertDisposition::Inserted { .. }
                | ConnectedInsertDisposition::AlreadyCovered { .. }
        ) {
            inserted_or_connected += 1;
            remaining_misses = remaining_misses.saturating_sub(1);
            if let Some(key) = result.stopping_key {
                if let Some(u) = graph.u_upper_bound(&key) {
                    if u > limits.optimize_above_u {
                        optimization_candidates.push((key, u, entry.representation_id.clone()));
                    }
                }
            }
        }
    }

    optimization_candidates
        .sort_unstable_by_key(|(key, u, id)| (std::cmp::Reverse(*u), *key, id.clone()));
    optimization_candidates.dedup_by_key(|(key, _, _)| *key);
    let mut optimizer_attempts = 0_usize;
    let mut optimizer_improvements = 0_usize;
    let mut optimizer_simulations = 0_u32;
    for (key, _, source_id) in optimization_candidates {
        let remaining = limits
            .total_optimizer_simulations
            .saturating_sub(optimizer_simulations);
        if remaining == 0 {
            break;
        }
        let item_budget = remaining.min(limits.per_item_optimizer_simulations);
        let run = optimize_vertex(
            graph,
            key,
            OptimizerProvenance {
                snapshot_id: snapshot_id.to_owned(),
                policy_model_id: oracle.model_id().to_owned(),
            },
            OptimizerBudget {
                max_depth: 3,
                max_states: 4096,
                max_simulations: item_budget,
                max_strands: 12,
                max_word_length: 96,
            },
            OptimizerPolicyProfile::GraphAware,
        )?;
        optimizer_attempts += 1;
        optimizer_simulations = optimizer_simulations
            .checked_add(run.simulations)
            .ok_or("RF campaign optimizer simulation overflow")?;
        optimizer_improvements += usize::from(run.success);
        manifest.push_str(&format!("optimizer_begin\t{source_id}\n"));
        manifest.push_str(&run.manifest_tsv());
        manifest.push_str(&format!("optimizer_end\t{source_id}\n"));
    }
    graph.recompute_unknot_routes()?;
    graph.recompute_acs10_routes()?;
    let after_u = graph.u_bounds();
    let before: std::collections::BTreeMap<_, _> = before_u.into_iter().collect();
    let total_u_decrease = after_u
        .iter()
        .filter_map(|(key, after)| {
            before
                .get(key)
                .map(|old| u64::from(old.saturating_sub(*after)))
        })
        .sum();
    let inserted_nodes = graph.node_count() - parent_nodes;
    let inserted_edges = graph.edge_count() - parent_edges;
    manifest.push_str(&format!(
        "summary\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
        corpus.len(),
        selected.len(),
        capacity_excluded,
        complete_preprocessing,
        initially_covered,
        initially_missing,
        insert_attempts,
        inserted_or_connected,
        remaining_misses,
        insert_simulations,
        policy_only_found,
        graph_assisted_found,
        optimizer_attempts,
        optimizer_improvements,
        optimizer_simulations,
        inserted_nodes,
        inserted_edges,
    ));
    manifest.push_str(&format!("total_u_decrease\t{total_u_decrease}\n"));
    Ok(RfCampaignRun {
        corpus_rows: corpus.len(),
        selected_rows: selected.len(),
        capacity_excluded,
        complete_preprocessing,
        initially_covered,
        initially_missing,
        insert_attempts,
        inserted_or_connected,
        remaining_misses,
        insert_simulations,
        policy_only_found,
        graph_assisted_found,
        optimizer_attempts,
        optimizer_improvements,
        optimizer_simulations,
        inserted_nodes,
        inserted_edges,
        total_u_decrease,
        manifest,
    })
}

fn optional_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_corpus_tsv() {
        let path =
            std::env::temp_dir().join(format!("unknotdb-rf-corpus-{}.tsv", std::process::id()));
        fs::write(
            &path,
            "representation_id\tpriority\tstrands\tword\troles\nbraid:x\t0\t1\t\ttest\n",
        )
        .unwrap();
        let rows = read_rf_corpus_tsv(&path).unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].representation.strands, 1);
        assert!(rows[0].representation.word.is_empty());
    }
}
