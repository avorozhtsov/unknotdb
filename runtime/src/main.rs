use std::env;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use unknotdb_runtime::{
    campaign::{read_rf_corpus_tsv, run_rf_campaign, RfCampaignLimits},
    catalogue_braid_import::import_catalogue_braid,
    catalogue_planar_import::import_catalogue_planar,
    census::{complete_natural_braids, enumerate_natural_braids, run_natural_braid_census},
    descending::{run_descending_backfill, DescendingBackfillLimits},
    embedding::export_embedding_pairs,
    expansion::{
        acs10_comparison_report, improve_quality, measure_b5_cohort, reverse_expand,
        verify_key_superset, B4RegressionReport, QualityLimits, ReverseLimits,
    },
    frontier::{
        expand_frontier_seed, FrontierDisposition, FrontierRunBatch, FrontierRunManifest,
        ScrambleLimits,
    },
    high_u::{
        improve_high_u_one_cc_branches, improve_high_u_with_policy, improve_high_u_with_templates,
        AnchoredTemplateProposal, HighUBranchLimits, HighUPolicyLimits, HighUTemplateLimits,
    },
    optimizer::{optimize_vertex, OptimizerBudget, OptimizerPolicyProfile, OptimizerProvenance},
    planar_import::{import_planar_cc0, import_planar_u1},
    policy::{
        preprocess_to_stopping_point, trace_policy_edge, ExternalPolicyOracle, PolicyLimits,
        PolicyOracle, POLICY_ADAPTER_VERSION,
    },
    population::{PopulationGraph, RelaxationOutcome},
    reducer::{DeterministicReducer, REDUCER_VERSION},
    representation::{
        BraidRepresentation, CheckpointedProofProgram, ProofProgram, SemanticAction,
        PACKED_REPRESENTATION_CODEC, VALIDATOR_VERSION,
    },
    rf_import::{import_best_traces, import_descending_corpus, read_best_trace_tsv},
    routing::{build_routing_sidecar, show_routing},
    supervision::{export_cc_frontier_supervision, export_graph_supervision},
    synthetic_edges, synthetic_key, synthetic_nodes,
    target_campaign::{read_targeted_braids_tsv, run_target_campaign, TargetCampaignLimits},
    targeted::{force_connected_insert, ConnectedInsertDisposition, ConnectedInsertLimits},
    wang_zhang_import::{import_wang_zhang_cube, import_wang_zhang_u1},
    write_snapshot_atomic, GraphSnapshot, HotRoute, PolicyStopAttestation, RepKey, Result,
    SnapshotMeta, SqliteSnapshot,
};

const USAGE: &str = r#"unknotdb-runtime - immutable graph snapshots and hot lookup

usage:
  unknotdb-runtime validate <snapshot.sqlite>
  unknotdb-runtime build-synthetic <snapshot.sqlite> [--nodes N]
  unknotdb-runtime normalize-braid <strands> <comma-separated-word> [--cyclic-band]
  unknotdb-runtime lookup-braid <snapshot.sqlite> <strands> <comma-separated-word>
      [--cyclic-band]
  unknotdb-runtime reduce-braid <strands> <comma-separated-word> [--cyclic-band]
  unknotdb-runtime preprocess-braid <strands> <comma-separated-word>
      [--max-policy-plies N] [--max-semantic-moves N]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime trace-edge-braid <strands> <comma-separated-word>
      [--max-policy-plies N] [--max-semantic-moves N]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime bootstrap-snapshot <snapshot.sqlite>
      [--max-policy-plies N] [--max-semantic-moves N]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime populate-unknot-frontier <snapshot.sqlite> --manifest <manifest.tsv>
      [--scramble-depth N] [--max-scramble-states N]
      [--max-scramble-candidates N] [--max-strands N] [--max-word-length N]
      [--min-scramble-cc N] [--max-scramble-cc N]
      [--max-policy-plies N] [--max-semantic-moves N]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime resume-frontier <input.sqlite> <output.sqlite>
      --manifest <manifest.tsv> [--generations N] [--seed-rank N]
      [--scramble-depth N] [--max-scramble-states N]
      [--max-scramble-candidates N] [--max-strands N] [--max-word-length N]
      [--min-scramble-cc N] [--max-scramble-cc N]
      [--max-policy-plies N] [--max-semantic-moves N]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime census-natural-braids <snapshot.sqlite>
      --report <report.md> --manifest <manifest.tsv>
      [--max-strands 2|3|4] [--max-word-length 10] [--scramble-depth 3]
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime enumerate-natural-braids
      [--max-strands 3] [--max-word-length 10]
  unknotdb-runtime complete-natural-braids <input.sqlite> <output.sqlite>
      --manifest <manifest.tsv> [--max-strands 2|3|4] [--max-word-length 10]
      [--max-policy-plies 16] [--max-semantic-moves 4]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime compact-snapshot <input.sqlite> <output.sqlite>
  unknotdb-runtime migrate-policy-adapter-v4 <input.sqlite> <output.sqlite>
      --manifest <manifest.tsv>
  unknotdb-runtime report-program-templates <snapshot.sqlite>
      --report <report.tsv> [--limit 100]
  unknotdb-runtime program-template-occurrences <snapshot.sqlite> <template-id>
      [--limit 100]
  unknotdb-runtime freeze-b4-regression <snapshot.sqlite>
      --seed-manifest <completion.tsv> --gate <gate.txt>
  unknotdb-runtime improve-quality <input.sqlite> <output.sqlite>
      --manifest <manifest.tsv> [--seed-limit N] [--max-depth N]
      [--max-states N] [--max-candidates N]
  unknotdb-runtime reverse-expand <input.sqlite> <output.sqlite>
      --manifest <manifest.tsv> [--seed-limit N] [frontier/policy bounds]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime selective-b5 <input.sqlite> <output.sqlite>
      --manifest <manifest.tsv> [--cohort-size N] [frontier/policy bounds]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime report-acs10 <snapshot.sqlite> --report <report.tsv>
  unknotdb-runtime build-routing-sidecar <snapshot.sqlite> <routing.sqlite>
  unknotdb-runtime show-routing <snapshot.sqlite> <routing.sqlite> <node-id-or-rep-key>
  unknotdb-runtime export-graph-supervision <snapshot.sqlite> <routing.sqlite> <dataset.sqlite>
  unknotdb-runtime export-cc-frontier-supervision <snapshot.sqlite> <routing.sqlite> <dataset.sqlite>
  unknotdb-runtime export-embedding-pairs <snapshot.sqlite> <identification.sqlite> <dataset.sqlite>
  unknotdb-runtime connect-braid <input.sqlite> <output.sqlite> <strands> <word>
      --manifest <manifest.tsv> --source-id <id>
      [--simulations 250] [--search-states 250] [--track-depth 12]
      [--max-result-u N]
      [--macro-depth 1] [--macro-states 1]
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime campaign-targeted-braids <input.sqlite> <output.sqlite>
      --cohort <targets.tsv> --manifest <manifest.tsv>
      [--per-item-simulations 1000] [--total-simulations 20000]
      [--track-depth 2] [--max-policy-plies 128] [--max-semantic-moves 32]
      [--macro-depth 1] [--macro-states 1]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime optimize-braid-node <input.sqlite> <output.sqlite> <strands> <word>
      --manifest <manifest.tsv> [--profile graph-aware|breadth-first|short-witness]
      [--max-depth 3] [--max-states 4096] [--simulations 250]
      [--max-strands 10] [--max-word-length 64]
  unknotdb-runtime campaign-rf-corpus <input.sqlite> <output.sqlite>
      --corpus <corpus.tsv> --manifest <manifest.tsv>
      [--max-priority 0] [--max-input-strands 12] [--max-input-word-length 48]
      [--max-insert-attempts 20] [--skip-missing 0]
      [--total-insert-simulations 5000] [--per-item-simulations 250]
      [--max-track-depth 12]
      [--total-optimizer-simulations 5000] [--per-item-optimizer-simulations 250]
      [--optimize-above-u 10] [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime improve-high-u-policy <input.sqlite> <output.sqlite>
      --manifest <manifest.tsv> [--min-u 1850] [--cohort-limit 32]
      [--keys-file PATH]
      [--per-node-simulations 250] [--total-simulations 8000]
      [--max-track-depth 32] [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime improve-high-u-branch <input.sqlite> <output.sqlite>
      --manifest <manifest.tsv> [--min-u 8] [--cohort-limit 2000]
      [--keys-file PATH]
      [--max-branches-per-node 64] [--branch-depth 1]
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime improve-high-u-templates <input.sqlite> <output.sqlite>
      --manifest <manifest.tsv> [--keys-file <keys.txt>]
      [--min-u 11] [--cohort-limit 978] [--max-templates 100]
      [--max-anchors-per-node 64]
  unknotdb-runtime descending-backfill <input.sqlite> <output.sqlite>
      --manifest <manifest.tsv> [--key HEX | --keys-file PATH]
      [--min-u 1850] [--cohort-limit 64]
      [--max-word-length 96] [--full-recompute-threshold 16]
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime import-planar-u1 <input.sqlite> <output.sqlite>
      --source-key HEX --cc-position N --certificate trace.json --manifest manifest.tsv
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime import-planar-cc0 <input.sqlite> <output.sqlite>
      --source-key HEX --certificate trace.json --manifest manifest.tsv
  unknotdb-runtime import-catalogue-planar <input.sqlite> <output.sqlite>
      --corpus witnesses.json [--corpus more.json] --manifest manifest.tsv
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime import-wang-zhang-u1 <input.sqlite> <output.sqlite>
      --corpus witnesses.json --manifest manifest.tsv
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime import-wang-zhang-cube <input.sqlite> <output.sqlite>
      --corpus witnesses.json --manifest manifest.tsv
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime import-catalogue-braid <input.sqlite> <output.sqlite>
      --cohort direct-braid.tsv --manifest manifest.tsv
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime import-rf-best <input.sqlite> <output.sqlite>
      --corpus <corpus.tsv> --best <best-witnesses.tsv> --manifest <manifest.tsv>
      [--only-representation-id ID]
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime import-rf-descending <input.sqlite> <output.sqlite>
      --corpus <corpus.tsv> --manifest <manifest.tsv> [--skip N] [--limit N]
      [--max-policy-plies 128] [--max-semantic-moves 32]
      --oracle <program> [oracle arguments...]
  unknotdb-runtime inspect <snapshot.sqlite>
  unknotdb-runtime bench <snapshot.sqlite> [--queries N] [--miss-percent N]
  unknotdb-runtime bench-population <snapshot.sqlite>
      [--duplicate-rounds N] [--refresh-rounds N] [--rebuild-rounds N]

`build-synthetic` creates workload data only. Synthetic edges are not proofs and
are marked as such in snapshot metadata. Production writers must pass programs
through the independent unknotdb verifier before calling the snapshot writer.
"#;

#[derive(Debug)]
struct Metrics {
    backend: &'static str,
    load: Duration,
    query_total: Duration,
    p50_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    query_count: usize,
    hits: u64,
    checksum: u64,
    rss_kib_after_load: Option<u64>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        print!("{USAGE}");
        return Ok(());
    };
    match command {
        "help" | "--help" | "-h" => print!("{USAGE}"),
        "build-synthetic" => {
            let path = positional_path(&args)?;
            let count = flag_u64(&args, "--nodes", 100_000)?;
            let count: u32 = count
                .try_into()
                .map_err(|_| "--nodes must fit in a 32-bit snapshot-local id")?;
            if count == 0 {
                return Err("--nodes must be at least 1".into());
            }
            let started = Instant::now();
            write_snapshot_atomic(
                &path,
                &SnapshotMeta::synthetic(),
                synthetic_nodes(count),
                synthetic_edges(count),
            )?;
            let bytes = std::fs::metadata(&path)?.len();
            println!(
                "built {} nodes / {} edges in {:.3}s: {} ({:.2} MiB)",
                count,
                count - 1,
                started.elapsed().as_secs_f64(),
                path.display(),
                bytes as f64 / (1024.0 * 1024.0)
            );
        }
        "normalize-braid" => {
            let input = braid_from_args(&args, "normalize-braid")?;
            let normalized = input.normalize()?;
            println!("mirrored:            {}", normalized.witness.mirrored);
            println!(
                "origin:              (x=0, y={})",
                normalized.witness.rotate_word_left
            );
            println!(
                "normalized word:     {}",
                format_word(&normalized.representation.word)
            );
            println!("representation hex: {}", hex(&normalized.encoded));
            println!("rep_key:             {}", hex(&normalized.key));
        }
        "lookup-braid" => {
            let path = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("lookup-braid needs a snapshot path")?;
            let input = braid_from_args(&args[1..], "lookup-braid")?;
            let normalized = input.normalize()?;
            let snapshot = GraphSnapshot::load_file(&path)?;
            println!("rep_key:             {}", hex(&normalized.key));
            println!("input mirrored:      {}", normalized.witness.mirrored);
            match snapshot.lookup(&normalized.key) {
                Some(route) => {
                    println!("hit:                 true");
                    println!("node_id:             {}", route.node_id);
                    println!("U upper bound:       {:?}", route.u_upper_bound);
                    println!("ACS10:               {:?}", route.acs10);
                    if let Some(step) = route.next_unknot {
                        let canonical = SemanticAction::decode_u63(step.first_action)?;
                        let input_action = canonical.normalized_to_input(
                            normalized.witness,
                            input.word.len().try_into().map_err(|_| "word too long")?,
                        )?;
                        println!("next target node:    {}", step.target_node);
                        println!("next edge CC cost:   {}", step.cc_cost);
                        println!("canonical action:    {canonical:?}");
                        println!("input action:        {input_action:?}");
                    } else {
                        println!("next action:         terminal");
                    }
                }
                None => println!("hit:                 false"),
            }
        }
        "reduce-braid" => {
            let input = braid_from_args(&args, "reduce-braid")?;
            let report = DeterministicReducer::reduce(&input)?;
            println!("reducer version:      {REDUCER_VERSION}");
            println!(
                "input origin:         (x=0, y={})",
                report.input_normalization.rotate_word_left
            );
            println!(
                "normalized source:    B{} [{}]",
                report.source.representation.strands,
                format_word(&report.source.representation.word)
            );
            println!(
                "L10:                  {} -> {}",
                report.l10_before, report.l10_after
            );
            println!("actions:              {}", report.program.actions.len());
            for (index, action) in report.program.actions.iter().enumerate() {
                println!("  {index}: {action:?}");
            }
            println!(
                "normalized output:    B{} [{}]",
                report.output.representation.strands,
                format_word(&report.output.representation.word)
            );
            println!("output rep_key:       {}", hex(&report.output.key));
            println!("program hex:          {}", hex(&report.program.encode()?));
        }
        "preprocess-braid" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("preprocess-braid needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs an executable path")?;
            let oracle_args = &args[oracle_index + 2..];
            let input = braid_from_args(command_args, "preprocess-braid")?;
            let limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            let report = preprocess_to_stopping_point(&input, &mut oracle, limits)?;
            println!("adapter version:       {POLICY_ADAPTER_VERSION}");
            println!("model id:              {}", report.model_id);
            println!("objective:             L{}", report.objective_ratio);
            println!("controller start:      {}", report.controller_initial_state);
            println!(
                "initial reducer:       {:?}",
                report.initial_reduction.program.actions
            );
            println!("rounds:                {}", report.rounds.len());
            for (index, round) in report.rounds.iter().enumerate() {
                println!(
                    "  {index}: source=B{} [{}]; policy={:?}",
                    round.source.representation.strands,
                    format_word(&round.source.representation.word),
                    round.decision
                );
            }
            println!("policy plies:          {}", report.policy_plies);
            println!("semantic moves:        {}", report.semantic_moves);
            println!("stop reason:           {:?}", report.stop_reason);
            println!(
                "preferred CC:          {:?}",
                report.preferred_crossing_change
            );
            println!(
                "normalized output:    B{} [{}]",
                report.output.representation.strands,
                format_word(&report.output.representation.word)
            );
            println!("output rep_key:       {}", hex(&report.output.key));
            println!("graph stopping point: {}", report.is_graph_stopping_point());
        }
        "trace-edge-braid" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("trace-edge-braid needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs an executable path")?;
            let oracle_args = &args[oracle_index + 2..];
            let source = braid_from_args(command_args, "trace-edge-braid")?;
            let limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            let edge = trace_policy_edge(&source, &mut oracle, limits)?;
            println!("model id:              {}", edge.model_id);
            println!("objective:             L{}", edge.objective_ratio);
            println!("source rep_key:        {}", hex(&edge.source.key));
            println!("crossing change:       {:?}", edge.crossing_change);
            println!("target rep_key:        {}", hex(&edge.target.key));
            println!(
                "target:                 B{} [{}]",
                edge.target.representation.strands,
                format_word(&edge.target.representation.word)
            );
            println!(
                "program version:       {}",
                CheckpointedProofProgram::VERSION
            );
            println!("validator version:     {VALIDATOR_VERSION}");
            println!("instructions:          {}", edge.program.instructions.len());
            for (index, instruction) in edge.program.instructions.iter().enumerate() {
                println!("  {index}: {instruction:?}");
            }
            println!("cc cost:               {}", edge.program.cc_cost());
            println!("program hex:           {}", hex(&edge.program.encode()?));
        }
        "bootstrap-snapshot" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("bootstrap-snapshot needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let path = positional_path(command_args)?;
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs an executable path")?;
            let oracle_args = &args[oracle_index + 2..];
            let limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            bootstrap_snapshot(&path, &mut oracle, limits)?;
            println!("published bootstrap snapshot: {}", path.display());
        }
        "populate-unknot-frontier" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("populate-unknot-frontier needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let path = positional_path(command_args)?;
            let manifest = flag_path(command_args, "--manifest")?;
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs an executable path")?;
            let oracle_args = &args[oracle_index + 2..];
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let scramble_limits = ScrambleLimits {
                depth: flag_u64(command_args, "--scramble-depth", 3)?
                    .try_into()
                    .map_err(|_| "--scramble-depth must fit u16")?,
                max_states: flag_u64(command_args, "--max-scramble-states", 512)?
                    .try_into()
                    .map_err(|_| "--max-scramble-states must fit u32")?,
                max_candidates: flag_u64(command_args, "--max-scramble-candidates", 64)?
                    .try_into()
                    .map_err(|_| "--max-scramble-candidates must fit u32")?,
                max_strands: flag_u64(command_args, "--max-strands", 3)?
                    .try_into()
                    .map_err(|_| "--max-strands must fit u16")?,
                max_word_length: flag_u64(command_args, "--max-word-length", 7)?
                    .try_into()
                    .map_err(|_| "--max-word-length must fit u32")?,
                min_cc_cost: flag_u64(command_args, "--min-scramble-cc", 1)?
                    .try_into()
                    .map_err(|_| "--min-scramble-cc must fit u8")?,
                max_cc_cost: flag_u64(command_args, "--max-scramble-cc", 1)?
                    .try_into()
                    .map_err(|_| "--max-scramble-cc must fit u8")?,
            };
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            populate_unknot_frontier(
                &path,
                &manifest,
                &mut oracle,
                policy_limits,
                scramble_limits,
            )?;
        }
        "resume-frontier" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("resume-frontier needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("resume-frontier needs an input snapshot path")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("resume-frontier needs an output snapshot path")?;
            let manifest = flag_path(command_args, "--manifest")?;
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs an executable path")?;
            let oracle_args = &args[oracle_index + 2..];
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let scramble_limits = scramble_limits_from_args(command_args)?;
            let generations: u32 = flag_u64(command_args, "--generations", 1)?
                .try_into()
                .map_err(|_| "--generations must fit u32")?;
            let seed_rank = optional_flag_u64(command_args, "--seed-rank")?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            resume_frontier(
                &input,
                &output,
                &manifest,
                &mut oracle,
                policy_limits,
                scramble_limits,
                generations,
                seed_rank,
            )?;
        }
        "census-natural-braids" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("census-natural-braids needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let snapshot_path = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("census-natural-braids needs a snapshot path")?;
            let report_path = flag_path(command_args, "--report")?;
            let manifest_path = flag_path(command_args, "--manifest")?;
            if report_path == manifest_path {
                return Err("census report and manifest paths must differ".into());
            }
            for output in [&report_path, &manifest_path] {
                if output.exists() {
                    return Err(
                        format!("census output already exists: {}", output.display()).into(),
                    );
                }
            }
            let max_strands: u16 = flag_u64(command_args, "--max-strands", 2)?
                .try_into()
                .map_err(|_| "--max-strands must fit u16")?;
            let max_word_length: u32 = flag_u64(command_args, "--max-word-length", 10)?
                .try_into()
                .map_err(|_| "--max-word-length must fit u32")?;
            let scramble_depth: u16 = flag_u64(command_args, "--scramble-depth", 3)?
                .try_into()
                .map_err(|_| "--scramble-depth must fit u16")?;
            let measurement_policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs an executable path")?;
            let oracle_args = &args[oracle_index + 2..];
            let cold = SqliteSnapshot::open_file(&snapshot_path)?;
            cold.verify_program_hashes()?;
            drop(cold);
            let snapshot = GraphSnapshot::load_file(&snapshot_path)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            let census = run_natural_braid_census(
                &mut oracle,
                &snapshot,
                max_strands,
                max_word_length,
                measurement_policy_limits,
                scramble_depth,
            )?;
            let manifest = census.manifest_tsv();
            let report = census.report_markdown();
            write_text_atomic(&manifest_path, &manifest)?;
            write_text_atomic(&report_path, &report)?;
            println!("published census report:   {}", report_path.display());
            println!("published census manifest: {}", manifest_path.display());
            println!(
                "raw_inputs={} normalized_seeds={} graph_hits={} policy_limits={}/{} scramble_limits=depth{},states{},candidates{},strands{},word{}",
                census.enumeration.raw_knot_inputs,
                census.enumeration.seeds.len(),
                census.seeds.iter().filter(|seed| seed.graph_hit).count(),
                census.recommended_policy_limits.max_policy_plies,
                census.recommended_policy_limits.max_semantic_moves,
                census.recommended_scramble_limits.depth,
                census.recommended_scramble_limits.max_states,
                census.recommended_scramble_limits.max_candidates,
                census.recommended_scramble_limits.max_strands,
                census.recommended_scramble_limits.max_word_length
            );
        }
        "enumerate-natural-braids" => {
            let max_strands: u16 = flag_u64(&args, "--max-strands", 3)?
                .try_into()
                .map_err(|_| "--max-strands must fit u16")?;
            let max_word_length: u32 = flag_u64(&args, "--max-word-length", 10)?
                .try_into()
                .map_err(|_| "--max-word-length must fit u32")?;
            let started = Instant::now();
            let enumeration = enumerate_natural_braids(max_strands, max_word_length)?;
            println!("max strands:          {}", enumeration.max_strands);
            println!("max word length:      {}", enumeration.max_word_length);
            println!("raw knot inputs:      {}", enumeration.raw_knot_inputs);
            println!("normalized seeds:     {}", enumeration.seeds.len());
            for strands in 1..=max_strands {
                let count = enumeration
                    .seeds
                    .iter()
                    .filter(|seed| seed.representation.representation.strands == strands)
                    .count();
                println!("normalized B{strands}:        {count}");
            }
            println!(
                "enumeration time:     {:.3}s",
                started.elapsed().as_secs_f64()
            );
        }
        "complete-natural-b2" | "complete-natural-braids" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("complete-natural-braids needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("complete-natural-braids needs an input snapshot path")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("complete-natural-braids needs an output snapshot path")?;
            let manifest_path = flag_path(command_args, "--manifest")?;
            if input == output {
                return Err(
                    "natural-braid completion input and output snapshots must differ".into(),
                );
            }
            for path in [&output, &manifest_path] {
                if path.exists() {
                    return Err(format!(
                        "natural-braid completion output already exists: {}",
                        path.display()
                    )
                    .into());
                }
            }
            let max_word_length: u32 = flag_u64(command_args, "--max-word-length", 10)?
                .try_into()
                .map_err(|_| "--max-word-length must fit u32")?;
            let max_strands: u16 = flag_u64(command_args, "--max-strands", 2)?
                .try_into()
                .map_err(|_| "--max-strands must fit u16")?;
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 16)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 4)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs an executable path")?;
            let oracle_args = &args[oracle_index + 2..];
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err(
                    "natural-braid completion parent and oracle policy model IDs differ".into(),
                );
            }
            let completion = complete_natural_braids(
                &mut population,
                &mut oracle,
                max_strands,
                max_word_length,
                policy_limits,
            )?;
            let manifest = completion.manifest_tsv(&parent_meta.source_generation);
            let inserted = completion.inserted_edges.len();
            let covered = completion.graph_hits_after;
            let (nodes, edges) = population.into_snapshot_records()?;
            let node_count = nodes.len();
            let edge_count = edges.len();
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!(
                    "complete-natural-b{}-length{}-v1-from-{}",
                    max_strands, max_word_length, parent_meta.source_generation
                ),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest_path, &manifest)?;
            println!(
                "published natural-braid-complete snapshot: {}",
                output.display()
            );
            println!("published completion manifest: {}", manifest_path.display());
            println!(
                "inserted={inserted} covered={covered}/{} nodes={node_count} edges={edge_count}",
                completion.normalized_seeds
            );
        }
        "freeze-b4-regression" => {
            let snapshot = positional_path(&args)?;
            let seed_manifest = flag_path(&args[1..], "--seed-manifest")?;
            let gate = flag_path(&args[1..], "--gate")?;
            if gate.exists() {
                return Err(format!("regression gate already exists: {}", gate.display()).into());
            }
            let started = Instant::now();
            let report = B4RegressionReport::run(&snapshot, &seed_manifest)?;
            write_text_atomic(&gate, &report.gate_text())?;
            println!("published B4 regression gate: {}", gate.display());
            println!(
                "coverage={}/{} replayed_edges={} U[p50/p95/max]={}/{}/{} total={:.3}s",
                report.covered_seeds,
                report.total_seeds,
                report.replayed_edges,
                report.u_p50,
                report.u_p95,
                report.u_max,
                started.elapsed().as_secs_f64()
            );
        }
        "improve-quality" => {
            let input = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("improve-quality needs input")?;
            let output = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("improve-quality needs output")?;
            let manifest = flag_path(&args[2..], "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let limits = QualityLimits {
                seed_limit: flag_u64(&args[2..], "--seed-limit", 128)? as usize,
                max_depth: flag_u64(&args[2..], "--max-depth", 3)?
                    .try_into()
                    .map_err(|_| "--max-depth must fit u16")?,
                max_states_per_depth: flag_u64(&args[2..], "--max-states", 4096)?
                    .try_into()
                    .map_err(|_| "--max-states must fit u32")?,
                max_candidates_per_depth: flag_u64(&args[2..], "--max-candidates", 256)?
                    .try_into()
                    .map_err(|_| "--max-candidates must fit u32")?,
                max_strands: flag_u64(&args[2..], "--max-strands", 5)?
                    .try_into()
                    .map_err(|_| "--max-strands must fit u16")?,
                max_word_length: flag_u64(&args[2..], "--max-word-length", 14)?
                    .try_into()
                    .map_err(|_| "--max-word-length must fit u32")?,
            };
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let run = improve_quality(&mut population, limits)?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!("quality-frontier-v0-from-{}", parent_meta.source_generation),
                parent_meta
                    .policy_model_id
                    .ok_or("parent has no policy model")?,
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!("published quality snapshot: {}", output.display());
            println!(
                "seeds={} candidates={} known_targets={} improved_nodes={} accepted_edges={} total_U_decrease={}",
                run.attempted_seeds,
                run.enumerated_candidates,
                run.known_target_candidates,
                run.improved_nodes,
                run.accepted_edges,
                run.total_u_decrease
            );
        }
        "reverse-expand" | "selective-b5" => {
            let selective_b5 = command == "selective-b5";
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("reverse expansion needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("reverse expansion needs input")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("reverse expansion needs output")?;
            let manifest = flag_path(&command_args[2..], "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 8)?
                    .try_into()
                    .map_err(|_| "policy plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 4)?
                    .try_into()
                    .map_err(|_| "semantic moves must fit u32")?,
            };
            let scramble_limits = ScrambleLimits {
                depth: flag_u64(command_args, "--scramble-depth", 2)?
                    .try_into()
                    .map_err(|_| "scramble depth must fit u16")?,
                max_states: flag_u64(command_args, "--max-scramble-states", 512)?
                    .try_into()
                    .map_err(|_| "scramble states must fit u32")?,
                max_candidates: flag_u64(command_args, "--max-scramble-candidates", 16)?
                    .try_into()
                    .map_err(|_| "scramble candidates must fit u32")?,
                max_strands: if selective_b5 { 5 } else { 4 },
                max_word_length: flag_u64(command_args, "--max-word-length", 12)?
                    .try_into()
                    .map_err(|_| "word length must fit u32")?,
                min_cc_cost: 1,
                max_cc_cost: 1,
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("parent and reverse-expansion oracle model IDs differ".into());
            }
            let started = Instant::now();
            let reverse = reverse_expand(
                &mut population,
                &mut oracle,
                ReverseLimits {
                    seed_limit: flag_u64(
                        command_args,
                        "--seed-limit",
                        if selective_b5 { 8 } else { 12 },
                    )? as usize,
                    min_seed_strands: if selective_b5 { 4 } else { 1 },
                    policy_limits,
                    scramble_limits,
                },
            )?;
            let mut manifest_text = reverse.manifest.clone();
            if selective_b5 {
                let cohort_size = flag_u64(command_args, "--cohort-size", 32)? as usize;
                let cohort =
                    measure_b5_cohort(&population, &mut oracle, policy_limits, cohort_size)?;
                manifest_text.push_str(&cohort.manifest);
            }
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!(
                    "{}-from-{}",
                    if selective_b5 {
                        "selective-b5-v0"
                    } else {
                        "bidirectional-v0"
                    },
                    parent_meta.source_generation
                ),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            let elapsed = started.elapsed();
            let bytes = std::fs::metadata(&output)?.len();
            manifest_text.push_str(&format!(
                "publication\t{}\t{}\t{:.6}\t{:.3}\t{:.3}\n",
                bytes,
                reverse.final_nodes - reverse.parent_nodes,
                elapsed.as_secs_f64(),
                (reverse.final_nodes - reverse.parent_nodes) as f64 / elapsed.as_secs_f64(),
                if reverse.final_nodes == reverse.parent_nodes {
                    0.0
                } else {
                    bytes as f64 / (reverse.final_nodes - reverse.parent_nodes) as f64
                }
            ));
            write_text_atomic(&manifest, &manifest_text)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!("published reverse snapshot: {}", output.display());
            println!(
                "nodes {}->{} edges {}->{} inserted={} improved={} acs10_candidates={} elapsed={:.3}s",
                reverse.parent_nodes,
                reverse.final_nodes,
                reverse.parent_edges,
                reverse.final_edges,
                reverse.inserted,
                reverse.improved,
                reverse.accepted_acs10,
                elapsed.as_secs_f64()
            );
        }
        "connect-braid" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("connect-braid needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("connect-braid needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("connect-braid needs an output snapshot")?;
            let manifest = flag_path(command_args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let representation = braid_from_args(&command_args[2..], "connect-braid")?;
            let source_id = flag_string(command_args, "--source-id")?;
            let limits = ConnectedInsertLimits {
                max_simulations: flag_u64(command_args, "--simulations", 250)?
                    .try_into()
                    .map_err(|_| "--simulations must fit u32")?,
                max_search_states: flag_u64(command_args, "--search-states", 250)?
                    .try_into()
                    .map_err(|_| "--search-states must fit u32")?,
                max_track_depth: flag_u64(command_args, "--track-depth", 12)?
                    .try_into()
                    .map_err(|_| "--track-depth must fit u16")?,
                max_result_u: optional_flag_string(command_args, "--max-result-u")?
                    .map(|value| value.parse())
                    .transpose()?,
                max_macro_semantic_depth: flag_u64(command_args, "--macro-depth", 1)?.try_into()?,
                max_macro_states: flag_u64(command_args, "--macro-states", 1)?.try_into()?,
                policy_limits: PolicyLimits {
                    max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                        .try_into()
                        .map_err(|_| "--max-policy-plies must fit u32")?,
                    max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                        .try_into()
                        .map_err(|_| "--max-semantic-moves must fit u32")?,
                },
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and connected-insert oracle model IDs differ".into());
            }
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let result = force_connected_insert(
                &mut population,
                &representation,
                &mut oracle,
                limits,
                &source_id,
                &snapshot_id,
            )?;
            write_text_atomic(&manifest, &result.manifest)?;
            match result.disposition {
                ConnectedInsertDisposition::Inserted { u_upper_bound } => {
                    let (nodes, edges) = population.into_snapshot_records()?;
                    let meta = SnapshotMeta::braid_mirror_orbit_v1(
                        format!(
                            "connected-insert-v0-{}-from-{}",
                            source_id, parent_meta.source_generation
                        ),
                        oracle.model_id(),
                    );
                    write_snapshot_atomic(&output, &meta, nodes, edges)?;
                    validate_published_snapshot(&output)?;
                    verify_key_superset(&input, &output)?;
                    println!(
                        "connected insert published: {} U={} nodes+={} edges+={} simulations={}",
                        output.display(),
                        u_upper_bound,
                        result.inserted_nodes,
                        result.inserted_edges,
                        result.simulations
                    );
                }
                ConnectedInsertDisposition::AlreadyCovered { u_upper_bound } => {
                    println!(
                        "already covered: U={} manifest={} (no snapshot published)",
                        u_upper_bound,
                        manifest.display()
                    );
                }
                ConnectedInsertDisposition::CoverageMiss { ref reason } => {
                    println!(
                        "coverage miss: {} manifest={} (no snapshot published)",
                        reason,
                        manifest.display()
                    );
                }
            }
        }
        "campaign-targeted-braids" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("campaign-targeted-braids needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("campaign-targeted-braids needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("campaign-targeted-braids needs an output snapshot")?;
            let cohort_path = flag_path(command_args, "--cohort")?;
            let manifest_path = flag_path(command_args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest_path])?;
            let limits = TargetCampaignLimits {
                per_item_simulations: flag_u64(command_args, "--per-item-simulations", 1000)?
                    .try_into()?,
                total_simulations: flag_u64(command_args, "--total-simulations", 20000)?
                    .try_into()?,
                max_track_depth: flag_u64(command_args, "--track-depth", 2)?.try_into()?,
                max_macro_semantic_depth: flag_u64(command_args, "--macro-depth", 1)?.try_into()?,
                max_macro_states: flag_u64(command_args, "--macro-states", 1)?.try_into()?,
                policy_limits: PolicyLimits {
                    max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                        .try_into()?,
                    max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                        .try_into()?,
                },
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let entries = read_targeted_braids_tsv(&cohort_path)?;
            let cohort_id = unknotdb::util::sha256_hex(&std::fs::read(&cohort_path)?);
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and target-campaign oracle model IDs differ".into());
            }
            let run = run_target_campaign(
                &mut population,
                &entries,
                &mut oracle,
                &snapshot_id,
                &cohort_id,
                limits,
            )?;
            write_text_atomic(&manifest_path, &run.manifest)?;
            if run.improved_to_target > 0 {
                let (nodes, edges) = population.into_snapshot_records()?;
                let meta = SnapshotMeta::braid_mirror_orbit_v1(
                    format!("target-campaign-v0-from-{}", parent_meta.source_generation),
                    oracle.model_id(),
                );
                write_snapshot_atomic(&output, &meta, nodes, edges)?;
                validate_published_snapshot(&output)?;
                verify_key_superset(&input, &output)?;
                println!(
                    "target campaign published: {} attempted={} improved={} misses={} nodes+={} edges+={} simulations={}",
                    output.display(), run.attempted, run.improved_to_target, run.misses,
                    run.inserted_nodes, run.inserted_edges, run.simulations,
                );
            } else {
                println!(
                    "target campaign no-improvement: attempted={} satisfied={} misses={} simulations={} manifest={} (no snapshot published)",
                    run.attempted, run.already_satisfied, run.misses, run.simulations,
                    manifest_path.display(),
                );
            }
        }
        "optimize-braid-node" => {
            let input = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("optimize-braid-node needs an input snapshot")?;
            let output = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("optimize-braid-node needs an output snapshot")?;
            let manifest = flag_path(&args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let representation = braid_from_args(&args[2..], "optimize-braid-node")?.normalize()?;
            let profile = match optional_flag_string(&args, "--profile")?.as_deref() {
                None | Some("graph-aware") => OptimizerPolicyProfile::GraphAware,
                Some("breadth-first") => OptimizerPolicyProfile::BreadthFirst,
                Some("short-witness") => OptimizerPolicyProfile::ShortWitness,
                Some(other) => return Err(format!("unknown optimizer profile `{other}`").into()),
            };
            let budget = OptimizerBudget {
                max_depth: flag_u64(&args, "--max-depth", 3)?
                    .try_into()
                    .map_err(|_| "--max-depth must fit u16")?,
                max_states: flag_u64(&args, "--max-states", 4096)?
                    .try_into()
                    .map_err(|_| "--max-states must fit u32")?,
                max_simulations: flag_u64(&args, "--simulations", 250)?
                    .try_into()
                    .map_err(|_| "--simulations must fit u32")?,
                max_strands: flag_u64(&args, "--max-strands", 10)?
                    .try_into()
                    .map_err(|_| "--max-strands must fit u16")?,
                max_word_length: flag_u64(&args, "--max-word-length", 64)?
                    .try_into()
                    .map_err(|_| "--max-word-length must fit u32")?,
            };
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let policy_model_id = parent_meta
                .policy_model_id
                .clone()
                .ok_or("optimizer snapshot has no policy model ID")?;
            let run = optimize_vertex(
                &mut population,
                representation.key,
                OptimizerProvenance {
                    snapshot_id,
                    policy_model_id: policy_model_id.clone(),
                },
                budget,
                profile,
            )?;
            write_text_atomic(&manifest, &run.manifest_tsv())?;
            if run.success {
                let (nodes, edges) = population.into_snapshot_records()?;
                let meta = SnapshotMeta::braid_mirror_orbit_v1(
                    format!("vertex-optimizer-v0-from-{}", parent_meta.source_generation),
                    policy_model_id,
                );
                write_snapshot_atomic(&output, &meta, nodes, edges)?;
                validate_published_snapshot(&output)?;
                verify_key_superset(&input, &output)?;
                println!(
                    "optimizer published: {} U {}->{} indirect={} simulations={}",
                    output.display(),
                    run.old_u,
                    run.new_u,
                    run.indirect_improvements,
                    run.simulations
                );
            } else {
                println!(
                    "no improvement: U={} manifest={} (no snapshot published)",
                    run.old_u,
                    manifest.display()
                );
            }
        }
        "campaign-rf-corpus" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("campaign-rf-corpus needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("campaign-rf-corpus needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("campaign-rf-corpus needs an output snapshot")?;
            let corpus_path = flag_path(command_args, "--corpus")?;
            let manifest = flag_path(command_args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let limits = RfCampaignLimits {
                max_priority: flag_u64(command_args, "--max-priority", 0)?
                    .try_into()
                    .map_err(|_| "--max-priority must fit u8")?,
                max_input_strands: flag_u64(command_args, "--max-input-strands", 12)?
                    .try_into()
                    .map_err(|_| "--max-input-strands must fit u16")?,
                max_input_word_length: flag_u64(command_args, "--max-input-word-length", 48)?
                    .try_into()
                    .map_err(|_| "--max-input-word-length must fit u32")?,
                max_insert_attempts: flag_u64(command_args, "--max-insert-attempts", 20)? as usize,
                skip_missing: flag_u64(command_args, "--skip-missing", 0)? as usize,
                total_insert_simulations: flag_u64(
                    command_args,
                    "--total-insert-simulations",
                    5000,
                )?
                .try_into()
                .map_err(|_| "--total-insert-simulations must fit u32")?,
                per_item_simulations: flag_u64(command_args, "--per-item-simulations", 250)?
                    .try_into()
                    .map_err(|_| "--per-item-simulations must fit u32")?,
                max_track_depth: flag_u64(command_args, "--max-track-depth", 12)?
                    .try_into()
                    .map_err(|_| "--max-track-depth must fit u16")?,
                total_optimizer_simulations: flag_u64(
                    command_args,
                    "--total-optimizer-simulations",
                    5000,
                )?
                .try_into()
                .map_err(|_| "--total-optimizer-simulations must fit u32")?,
                per_item_optimizer_simulations: flag_u64(
                    command_args,
                    "--per-item-optimizer-simulations",
                    250,
                )?
                .try_into()
                .map_err(|_| "--per-item-optimizer-simulations must fit u32")?,
                optimize_above_u: flag_u64(command_args, "--optimize-above-u", 10)?
                    .try_into()
                    .map_err(|_| "--optimize-above-u must fit u32")?,
                policy_limits: PolicyLimits {
                    max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                        .try_into()
                        .map_err(|_| "--max-policy-plies must fit u32")?,
                    max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                        .try_into()
                        .map_err(|_| "--max-semantic-moves must fit u32")?,
                },
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let corpus = read_rf_corpus_tsv(&corpus_path)?;
            let corpus_id = unknotdb::util::sha256_hex(&std::fs::read(&corpus_path)?);
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and RF campaign oracle model IDs differ".into());
            }
            let run = run_rf_campaign(
                &mut population,
                &corpus,
                &mut oracle,
                &snapshot_id,
                &corpus_id,
                limits,
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!(
                    "rf-targeted-campaign-v0-from-{}",
                    parent_meta.source_generation
                ),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "RF campaign published: {} selected={} initial={}/{} inserted_or_connected={} remaining_misses={} optimizer_improvements={} nodes+={} edges+={} simulations={}/{}",
                output.display(),
                run.selected_rows,
                run.initially_covered,
                run.initially_missing,
                run.inserted_or_connected,
                run.remaining_misses,
                run.optimizer_improvements,
                run.inserted_nodes,
                run.inserted_edges,
                run.insert_simulations,
                run.optimizer_simulations,
            );
        }
        "improve-high-u-policy" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("improve-high-u-policy needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("improve-high-u-policy needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("improve-high-u-policy needs an output snapshot")?;
            let manifest = flag_path(command_args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let limits = HighUPolicyLimits {
                min_u: flag_u64(command_args, "--min-u", 1850)?
                    .try_into()
                    .map_err(|_| "--min-u must fit u32")?,
                cohort_limit: flag_u64(command_args, "--cohort-limit", 32)? as usize,
                per_node_simulations: flag_u64(command_args, "--per-node-simulations", 250)?
                    .try_into()
                    .map_err(|_| "--per-node-simulations must fit u32")?,
                total_simulations: flag_u64(command_args, "--total-simulations", 8000)?
                    .try_into()
                    .map_err(|_| "--total-simulations must fit u32")?,
                max_track_depth: flag_u64(command_args, "--max-track-depth", 32)?
                    .try_into()
                    .map_err(|_| "--max-track-depth must fit u16")?,
                policy_limits: PolicyLimits {
                    max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                        .try_into()
                        .map_err(|_| "--max-policy-plies must fit u32")?,
                    max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                        .try_into()
                        .map_err(|_| "--max-semantic-moves must fit u32")?,
                },
            };
            let selected_keys = optional_flag_string(command_args, "--keys-file")?
                .map(|path| read_rep_keys(Path::new(&path)))
                .transpose()?;
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and high-U oracle model IDs differ".into());
            }
            let run = improve_high_u_with_policy(
                &mut population,
                &mut oracle,
                &snapshot_id,
                limits,
                selected_keys.as_deref(),
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!(
                    "high-u-l1000-policy-v0-from-{}",
                    parent_meta.source_generation
                ),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "high-U policy snapshot: {} selected={} improved={} excluded={} collateral={} U_decrease={} nodes+={} edges+={} simulations={}",
                output.display(),
                run.selected,
                run.improved,
                run.excluded,
                run.collateral_improvements,
                run.total_selected_u_decrease,
                run.inserted_nodes,
                run.inserted_edges,
                run.total_simulations,
            );
        }
        "improve-high-u-branch" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("improve-high-u-branch needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("improve-high-u-branch needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("improve-high-u-branch needs an output snapshot")?;
            let manifest = flag_path(command_args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let min_u: u32 = flag_u64(command_args, "--min-u", 8)?
                .try_into()
                .map_err(|_| "--min-u must fit u32")?;
            let cohort_limit = flag_u64(command_args, "--cohort-limit", 2000)? as usize;
            let max_branches = flag_u64(command_args, "--max-branches-per-node", 64)? as usize;
            let branch_depth: u16 = flag_u64(command_args, "--branch-depth", 1)?
                .try_into()
                .map_err(|_| "--branch-depth must fit u16")?;
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let selected_keys = optional_flag_string(command_args, "--keys-file")?
                .map(|path| read_rep_keys(Path::new(&path)))
                .transpose()?;
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and one-CC branch oracle model IDs differ".into());
            }
            let run = improve_high_u_one_cc_branches(
                &mut population,
                &mut oracle,
                &snapshot_id,
                HighUBranchLimits {
                    min_u,
                    cohort_limit,
                    max_branches_per_node: max_branches,
                    max_depth: branch_depth,
                    policy_limits,
                },
                selected_keys.as_deref(),
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!(
                    "high-u-one-cc-branch-v0-from-{}",
                    parent_meta.source_generation
                ),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "high-U branch snapshot: {} selected={} improved={} excluded={} branches={} hits={} collateral={} edges+={}",
                output.display(),
                run.selected,
                run.improved,
                run.excluded,
                run.evaluated_branches,
                run.exact_graph_hits,
                run.collateral_improvements,
                run.inserted_edges,
            );
        }
        "improve-high-u-templates" => {
            let input = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("improve-high-u-templates needs an input snapshot")?;
            let output = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("improve-high-u-templates needs an output snapshot")?;
            let manifest = flag_path(&args[2..], "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let min_u: u32 = flag_u64(&args[2..], "--min-u", 11)?.try_into()?;
            let cohort_limit = flag_u64(&args[2..], "--cohort-limit", 978)? as usize;
            let max_templates = flag_u64(&args[2..], "--max-templates", 100)? as usize;
            let max_anchors = flag_u64(&args[2..], "--max-anchors-per-node", 64)? as usize;
            let selected_keys = optional_flag_string(&args[2..], "--keys-file")?
                .map(|path| read_rep_keys(Path::new(&path)))
                .transpose()?;
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let cold = SqliteSnapshot::open_file(&input)?;
            if cold.schema_version()? < 3 {
                return Err("anchored-template improvement requires schema version 3".into());
            }
            let mut stmt = cold.connection().prepare(
                "SELECT p.program_id,count(*),p.program \
                 FROM programs p JOIN edges e ON e.program_id=p.program_id \
                 GROUP BY p.program_id HAVING min(e.cc_cost)=1 AND max(e.cc_cost)=1 \
                 ORDER BY count(*) DESC,p.program_id ASC LIMIT ?1",
            )?;
            let templates = stmt
                .query_map([i64::try_from(max_templates)?], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                })?
                .map(|row| -> Result<AnchoredTemplateProposal> {
                    let (id, uses, blob) = row?;
                    Ok(AnchoredTemplateProposal {
                        template_id: id.try_into()?,
                        uses: uses.try_into()?,
                        program: CheckpointedProofProgram::decode(&blob)?,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            drop(stmt);
            drop(cold);
            let (mut population, mut meta) = PopulationGraph::from_snapshot(&input)?;
            let run = improve_high_u_with_templates(
                &mut population,
                &templates,
                &snapshot_id,
                selected_keys.as_deref(),
                HighUTemplateLimits {
                    min_u,
                    cohort_limit,
                    max_templates,
                    max_anchors_per_node: max_anchors,
                },
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            meta.source_generation = format!(
                "high-u-anchored-template-v0-from-{}",
                meta.source_generation
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "high-U template snapshot: {} selected={} improved={} bindings={} legal={} hits={} edges+={}",
                output.display(), run.selected, run.improved, run.evaluated_bindings,
                run.legal_bindings, run.exact_graph_hits, run.inserted_edges,
            );
        }
        "import-rf-best" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("import-rf-best needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("import-rf-best needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("import-rf-best needs an output snapshot")?;
            let corpus_path = flag_path(command_args, "--corpus")?;
            let best_path = flag_path(command_args, "--best")?;
            let manifest = flag_path(command_args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let corpus = read_rf_corpus_tsv(&corpus_path)?;
            let mut traces = read_best_trace_tsv(&best_path)?;
            if let Some(representation_id) =
                optional_flag_string(command_args, "--only-representation-id")?
            {
                traces.retain(|trace| trace.representation_id() == representation_id);
                if traces.is_empty() {
                    return Err(format!(
                        "RF best-witness file has no trace for `{representation_id}`"
                    )
                    .into());
                }
            }
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and RF best-trace oracle model IDs differ".into());
            }
            let run = import_best_traces(
                &mut population,
                &corpus,
                &traces,
                &mut oracle,
                &snapshot_id,
                policy_limits,
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!("rf-best-traces-v0-from-{}", parent_meta.source_generation),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "RF best-trace snapshot: {} selected={} covered={} imported={} improved={} failed={} nodes+={} edges+={}",
                output.display(), run.selected, run.initially_covered, run.imported,
                run.improved, run.failed, run.inserted_nodes, run.inserted_edges,
            );
        }
        "import-rf-descending" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("import-rf-descending needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("import-rf-descending needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("import-rf-descending needs an output snapshot")?;
            let corpus_path = flag_path(command_args, "--corpus")?;
            let manifest = flag_path(command_args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let skip = flag_u64(command_args, "--skip", 0)? as usize;
            let limit = flag_u64(command_args, "--limit", u64::MAX)? as usize;
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let corpus = read_rf_corpus_tsv(&corpus_path)?;
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and RF descending oracle model IDs differ".into());
            }
            let run = import_descending_corpus(
                &mut population,
                &corpus,
                &mut oracle,
                &snapshot_id,
                policy_limits,
                skip,
                limit,
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!("rf-descending-v0-from-{}", parent_meta.source_generation),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "RF descending snapshot: {} selected={} covered={} imported={} failed={} nodes+={} edges+={}",
                output.display(), run.selected, run.initially_covered, run.imported,
                run.failed, run.inserted_nodes, run.inserted_edges,
            );
        }
        "import-planar-u1" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("import-planar-u1 needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("import-planar-u1 needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("import-planar-u1 needs an output snapshot")?;
            let manifest = flag_path(command_args, "--manifest")?;
            let certificate_path = flag_path(command_args, "--certificate")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let source_key = parse_rep_key(&flag_string(command_args, "--source-key")?)?;
            let crossing_position: u32 = flag_u64(command_args, "--cc-position", u64::MAX)?
                .try_into()
                .map_err(|_| "--cc-position must fit u32")?;
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let certificate = std::fs::read(&certificate_path)?;
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and planar-import oracle model IDs differ".into());
            }
            let run = import_planar_u1(
                &mut population,
                source_key,
                crossing_position,
                certificate,
                &mut oracle,
                policy_limits,
                &snapshot_id,
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!("planar-riii-v0-from-{}", parent_meta.source_generation),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "planar U1 snapshot: {} old_u={} new_u={} nodes+={} edges+={} certificate={}",
                output.display(),
                run.old_u,
                run.new_u,
                run.inserted_nodes,
                run.inserted_edges,
                hex(&run.certificate_id),
            );
        }
        "import-planar-cc0" => {
            let input = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("import-planar-cc0 needs an input snapshot")?;
            let output = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("import-planar-cc0 needs an output snapshot")?;
            let manifest = flag_path(&args, "--manifest")?;
            let certificate_path = flag_path(&args, "--certificate")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let source_key = parse_rep_key(&flag_string(&args, "--source-key")?)?;
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let certificate = std::fs::read(&certificate_path)?;
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let run = import_planar_cc0(&mut population, source_key, certificate, &snapshot_id)?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!("planar-cc0-v0-from-{}", parent_meta.source_generation),
                parent_meta
                    .policy_model_id
                    .as_deref()
                    .ok_or("planar CC0 parent snapshot has no policy model ID")?,
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "planar CC0 snapshot: {} old_u={} new_u={} nodes+={} edges+={} certificate={}",
                output.display(),
                run.old_u,
                run.new_u,
                run.inserted_nodes,
                run.inserted_edges,
                hex(&run.certificate_id),
            );
        }
        "import-wang-zhang-u1" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("import-wang-zhang-u1 needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("import-wang-zhang-u1 needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("import-wang-zhang-u1 needs an output snapshot")?;
            let manifest = flag_path(command_args, "--manifest")?;
            let corpus_path = flag_path(command_args, "--corpus")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let corpus = std::fs::read(&corpus_path)?;
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and Wang--Zhang oracle model IDs differ".into());
            }
            let run = import_wang_zhang_u1(
                &mut population,
                &corpus,
                &mut oracle,
                policy_limits,
                &snapshot_id,
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!("wang-zhang-u1-v0-from-{}", parent_meta.source_generation),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "Wang--Zhang U1 snapshot: {} rows={} nodes+={} edges+={} certificates+={} collateral={}",
                output.display(), run.rows, run.inserted_nodes, run.inserted_edges,
                run.inserted_certificates, run.collateral_improvements,
            );
        }
        "import-wang-zhang-cube" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("import-wang-zhang-cube needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("import-wang-zhang-cube needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("import-wang-zhang-cube needs an output snapshot")?;
            let manifest = flag_path(command_args, "--manifest")?;
            let corpus_path = flag_path(command_args, "--corpus")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let corpus = std::fs::read(&corpus_path)?;
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and Wang--Zhang oracle model IDs differ".into());
            }
            let run = import_wang_zhang_cube(
                &mut population,
                &corpus,
                &mut oracle,
                policy_limits,
                &snapshot_id,
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!("wang-zhang-cube-v0-from-{}", parent_meta.source_generation),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "Wang--Zhang cube snapshot: {} states={} nodes+={} edges+={} certificates+={} collateral={}",
                output.display(), run.states, run.inserted_nodes, run.inserted_edges,
                run.inserted_certificates, run.collateral_improvements,
            );
        }
        "import-catalogue-planar" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("import-catalogue-planar needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("import-catalogue-planar needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("import-catalogue-planar needs an output snapshot")?;
            let manifest = flag_path(command_args, "--manifest")?;
            let corpus_paths: Vec<PathBuf> = command_args
                .windows(2)
                .filter(|pair| pair[0] == "--corpus")
                .map(|pair| PathBuf::from(&pair[1]))
                .collect();
            if corpus_paths.is_empty() {
                return Err("import-catalogue-planar needs at least one --corpus".into());
            }
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let corpus_storage: Vec<(String, Vec<u8>)> = corpus_paths
                .iter()
                .map(|path| -> Result<_> { Ok((path.display().to_string(), std::fs::read(path)?)) })
                .collect::<Result<_>>()?;
            let corpus_refs: Vec<(&str, &[u8])> = corpus_storage
                .iter()
                .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
                .collect();
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and catalogue-planar oracle model IDs differ".into());
            }
            let run = import_catalogue_planar(
                &mut population,
                &corpus_refs,
                &mut oracle,
                policy_limits,
                &snapshot_id,
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!(
                    "catalogue-planar-chain-v0-from-{}",
                    parent_meta.source_generation
                ),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "catalogue planar snapshot: {} corpus={} witnesses={} candidates={} improved={} already={} absent={} newly_connected={} failed={} direct={} collateral={} nodes+={} edges+={} certificates+={}",
                output.display(), run.corpus_entries, run.witness_entries,
                run.strict_candidates, run.improved, run.already_sufficient,
                run.absent_sources, run.newly_connected_sources, run.failed,
                run.direct_improvements,
                run.collateral_improvements, run.inserted_nodes, run.inserted_edges,
                run.inserted_certificates,
            );
        }
        "import-catalogue-braid" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("import-catalogue-braid needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("import-catalogue-braid needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("import-catalogue-braid needs an output snapshot")?;
            let cohort_path = flag_path(command_args, "--cohort")?;
            let manifest = flag_path(command_args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let policy_limits = PolicyLimits {
                max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                    .try_into()
                    .map_err(|_| "--max-policy-plies must fit u32")?,
                max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                    .try_into()
                    .map_err(|_| "--max-semantic-moves must fit u32")?,
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let cohort = std::fs::read(&cohort_path)?;
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and catalogue-braid oracle model IDs differ".into());
            }
            let run = import_catalogue_braid(
                &mut population,
                &cohort,
                &mut oracle,
                policy_limits,
                &snapshot_id,
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!(
                    "catalogue-direct-braid-v0-from-{}",
                    parent_meta.source_generation
                ),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "catalogue braid snapshot: {} selected={} improved={} absent_source={} already={} failed={} collateral={} nodes+={} edges+={}",
                output.display(), run.selected, run.improved, run.absent_sources,
                run.already_sufficient, run.failed, run.collateral_improvements,
                run.inserted_nodes, run.inserted_edges,
            );
        }
        "descending-backfill" => {
            let oracle_index = args
                .iter()
                .position(|arg| arg == "--oracle")
                .ok_or("descending-backfill needs --oracle <program> [arguments...]")?;
            let command_args = &args[..oracle_index];
            let input = command_args
                .get(1)
                .map(PathBuf::from)
                .ok_or("descending-backfill needs an input snapshot")?;
            let output = command_args
                .get(2)
                .map(PathBuf::from)
                .ok_or("descending-backfill needs an output snapshot")?;
            let manifest = flag_path(command_args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let limits = DescendingBackfillLimits {
                min_u: flag_u64(command_args, "--min-u", 1850)?
                    .try_into()
                    .map_err(|_| "--min-u must fit u32")?,
                cohort_limit: flag_u64(command_args, "--cohort-limit", 64)? as usize,
                max_word_length: flag_u64(command_args, "--max-word-length", 96)?
                    .try_into()
                    .map_err(|_| "--max-word-length must fit u32")?,
                full_recompute_threshold: flag_u64(command_args, "--full-recompute-threshold", 16)?
                    as usize,
                policy_limits: PolicyLimits {
                    max_policy_plies: flag_u64(command_args, "--max-policy-plies", 128)?
                        .try_into()
                        .map_err(|_| "--max-policy-plies must fit u32")?,
                    max_semantic_moves: flag_u64(command_args, "--max-semantic-moves", 32)?
                        .try_into()
                        .map_err(|_| "--max-semantic-moves must fit u32")?,
                },
            };
            let only_key = optional_flag_string(command_args, "--key")?
                .map(|value| parse_rep_key(&value))
                .transpose()?;
            let keys_file = optional_flag_string(command_args, "--keys-file")?;
            if only_key.is_some() && keys_file.is_some() {
                return Err("use only one of --key and --keys-file".into());
            }
            let selected_keys = if let Some(key) = only_key {
                Some(vec![key])
            } else if let Some(path) = keys_file {
                Some(read_rep_keys(Path::new(&path))?)
            } else {
                None
            };
            let oracle_program = args
                .get(oracle_index + 1)
                .ok_or("--oracle needs a program")?;
            let oracle_args = &args[oracle_index + 2..];
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let mut oracle = ExternalPolicyOracle::spawn(Path::new(oracle_program), oracle_args)?;
            if parent_meta.policy_model_id.as_deref() != Some(oracle.model_id()) {
                return Err("snapshot and descending oracle model IDs differ".into());
            }
            let run = run_descending_backfill(
                &mut population,
                &mut oracle,
                &snapshot_id,
                limits,
                selected_keys.as_deref(),
            )?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!("descending-artin-v0-from-{}", parent_meta.source_generation),
                oracle.model_id(),
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            write_text_atomic(&manifest, &run.manifest)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            println!(
                "descending snapshot: {} selected={} certified={} excluded={} failed={} direct={} collateral={} nodes+={} edges+={}",
                output.display(), run.selected, run.certified, run.excluded, run.failed,
                run.direct_improvements, run.collateral_improvements,
                run.inserted_nodes, run.inserted_edges,
            );
        }
        "report-acs10" => {
            let snapshot = positional_path(&args)?;
            let report = flag_path(&args[1..], "--report")?;
            if report.exists() {
                return Err(format!("ACS10 report already exists: {}", report.display()).into());
            }
            let text = acs10_comparison_report(
                &snapshot,
                flag_u64(&args[1..], "--sample-limit", 100)? as usize,
            )?;
            write_text_atomic(&report, &text)?;
            println!(
                "published Action-Choice Score 10 report: {}",
                report.display()
            );
        }
        "build-routing-sidecar" => {
            let proof = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("build-routing-sidecar needs a proof snapshot")?;
            let output = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("build-routing-sidecar needs an output sidecar")?;
            let report = build_routing_sidecar(&proof, &output)?;
            println!(
                "published routing sidecar: {} nodes={} edges={} terminals={} frontier_rows={} shortest_next_diff={} shortest_improved={} semantic_saving={} max_semantic_saving={} l1000_next_diff={} l1000_more_cc={} bytes={} proof_sha256={}",
                output.display(),
                report.nodes,
                report.edges,
                report.terminals,
                report.frontier_rows,
                report.shortest_next_differs_from_active,
                report.shortest_improves_active,
                report.aggregate_semantic_saving,
                report.maximum_semantic_saving,
                report.l1000_next_differs_from_shortest,
                report.l1000_uses_more_cc,
                report.bytes,
                report.proof_sha256,
            );
        }
        "show-routing" => {
            let proof = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("show-routing needs a proof snapshot")?;
            let sidecar = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("show-routing needs a routing sidecar")?;
            let identifier = args
                .get(3)
                .ok_or("show-routing needs a node ID or representation key")?;
            print!("{}", show_routing(&proof, &sidecar, identifier)?);
        }
        "export-graph-supervision" => {
            let proof = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("export-graph-supervision needs a proof snapshot")?;
            let routing = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("export-graph-supervision needs a routing sidecar")?;
            let output = args
                .get(3)
                .map(PathBuf::from)
                .ok_or("export-graph-supervision needs an output dataset")?;
            if args.len() != 4 {
                return Err("export-graph-supervision takes exactly three paths".into());
            }
            let report = export_graph_supervision(&proof, &routing, &output)?;
            println!(
                "published graph supervision: {} selected_edges={} replayed_edges={} planar_excluded={} theorem_actions_excluded={} preprocessor_labels={} cc_labels={} occurrences={} conflicts={} split={}/{}/{} bytes={} proof_sha256={} routing_sha256={}",
                output.display(),
                report.selected_edges,
                report.replayed_edges,
                report.excluded_planar_edges,
                report.excluded_theorem_actions,
                report.preprocessor_labels,
                report.cc_labels,
                report.occurrences,
                report.conflicting_states,
                report.train_labels,
                report.validation_labels,
                report.test_labels,
                report.bytes,
                report.proof_sha256,
                report.routing_sha256,
            );
        }
        "export-cc-frontier-supervision" => {
            let proof = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("export-cc-frontier-supervision needs a proof snapshot")?;
            let routing = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("export-cc-frontier-supervision needs a routing sidecar")?;
            let output = args
                .get(3)
                .map(PathBuf::from)
                .ok_or("export-cc-frontier-supervision needs an output dataset")?;
            if args.len() != 4 {
                return Err("export-cc-frontier-supervision takes exactly three paths".into());
            }
            let report = export_cc_frontier_supervision(&proof, &routing, &output)?;
            println!(
                "published CC frontier supervision: {} candidate_edges={} replayed_edges={} states={} options={} accepted={} compared={} multi_accepted_states={} bytes={} proof_sha256={} routing_sha256={}",
                output.display(),
                report.candidate_edges,
                report.replayed_edges,
                report.states,
                report.options,
                report.accepted_options,
                report.compared_options,
                report.multi_accepted_states,
                report.bytes,
                report.proof_sha256,
                report.routing_sha256,
            );
        }
        "export-embedding-pairs" => {
            let proof = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("export-embedding-pairs needs a proof snapshot")?;
            let identification = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("export-embedding-pairs needs an identification sidecar")?;
            let output = args
                .get(3)
                .map(PathBuf::from)
                .ok_or("export-embedding-pairs needs an output dataset")?;
            if args.len() != 4 {
                return Err("export-embedding-pairs takes exactly three paths".into());
            }
            let report = export_embedding_pairs(&proof, &identification, &output)?;
            println!(
                "published embedding pairs: {} replayed_edges={} representations={} pairs={} cc=0:{}/1:{}/upper1:{} rm=0:{}/1:{}/2:{} split={}/{}/{} bytes={} proof_sha256={} identification_sha256={}",
                output.display(), report.replayed_edges, report.representations, report.pairs,
                report.exact_cc0, report.exact_cc1, report.upper_cc1, report.exact_rm0,
                report.exact_rm1, report.exact_rm2, report.split_train,
                report.split_validation, report.split_test, report.bytes,
                report.proof_sha256, report.identification_sha256,
            );
        }
        "compact-snapshot" => {
            let input = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("compact-snapshot needs an input path")?;
            let output = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("compact-snapshot needs an output path")?;
            if input == output {
                return Err("compact-snapshot output must differ from input".into());
            }
            let started = Instant::now();
            let (population, mut meta) = PopulationGraph::from_snapshot(&input)?;
            meta.source_generation = format!(
                "compact-schema{}-from-{}",
                unknotdb_runtime::SCHEMA_VERSION,
                meta.source_generation
            );
            meta.representation_codec = PACKED_REPRESENTATION_CODEC.into();
            let (nodes, edges) = population.into_snapshot_records()?;
            let node_count = nodes.len();
            let edge_count = edges.len();
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            let bytes = std::fs::metadata(&output)?.len();
            println!(
                "compacted {node_count} nodes / {edge_count} edges in {:.3}s: {} ({:.2} MiB)",
                started.elapsed().as_secs_f64(),
                output.display(),
                bytes as f64 / (1024.0 * 1024.0)
            );
        }
        "migrate-policy-adapter-v4" => {
            let input = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("migrate-policy-adapter-v4 needs an input snapshot")?;
            let output = args
                .get(2)
                .map(PathBuf::from)
                .ok_or("migrate-policy-adapter-v4 needs an output snapshot")?;
            let manifest = flag_path(&args, "--manifest")?;
            ensure_new_outputs(&input, &[&output, &manifest])?;
            let snapshot_id = unknotdb::util::sha256_hex(&std::fs::read(&input)?);
            let (mut population, parent_meta) = PopulationGraph::from_snapshot(&input)?;
            let old_adapter = parent_meta
                .policy_adapter_version
                .as_deref()
                .ok_or("parent snapshot has no policy adapter")?;
            if old_adapter
                != "initial-reducer-clean-controller-top1-self-loop-skip4-mirror-orbit-v3"
            {
                return Err("conservative migration requires the exact v3 parent adapter".into());
            }
            let (terminal, preferred_cc) =
                population.migrate_conservative_policy_adapter(old_adapter, POLICY_ADAPTER_VERSION);
            let model_id = parent_meta
                .policy_model_id
                .as_deref()
                .ok_or("parent snapshot has no policy model")?;
            let (nodes, edges) = population.into_snapshot_records()?;
            let node_count = nodes.len();
            let edge_count = edges.len();
            let meta = SnapshotMeta::braid_mirror_orbit_v1(
                format!(
                    "policy-adapter-v4-conservative-migration-from-{}",
                    snapshot_id
                ),
                model_id,
            );
            write_snapshot_atomic(&output, &meta, nodes, edges)?;
            validate_published_snapshot(&output)?;
            verify_key_superset(&input, &output)?;
            let body = format!(
                "unknotdb-policy-adapter-migration-v0\nparent_snapshot_sha256\t{}\nold_adapter\t{}\nnew_adapter\t{}\nproof_rule\tnew adapter differs only after a controller cycle; stored v3 graph nodes are exclusively terminal or preferred-CC stops, so that branch is unreachable\nnodes\t{}\nterminal_reattested\t{}\npreferred_cc_reattested\t{}\nedges_replayed\t{}\n",
                snapshot_id,
                old_adapter,
                POLICY_ADAPTER_VERSION,
                node_count,
                terminal,
                preferred_cc,
                edge_count,
            );
            write_text_atomic(&manifest, &body)?;
            println!(
                "policy adapter v4 snapshot: {} nodes={} terminal={} preferred_cc={} edges={}",
                output.display(),
                node_count,
                terminal,
                preferred_cc,
                edge_count
            );
        }
        "report-program-templates" => {
            let snapshot = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("report-program-templates needs a snapshot path")?;
            let report = flag_path(&args[1..], "--report")?;
            if report.exists() {
                return Err(format!("template report already exists: {}", report.display()).into());
            }
            let limit = flag_u64(&args[1..], "--limit", 100)?;
            let cold = SqliteSnapshot::open_file(&snapshot)?;
            if cold.schema_version()? < 3 {
                return Err("program templates require schema version 3".into());
            }
            let mut text = String::from(
                "template_id\tuses\tdistinct_anchors\tmin_anchor_x\tmax_anchor_x\tmin_anchor_y\tmax_anchor_y\tcc_cost\tinstructions\texample_edge\tsha256\ttemplate\n",
            );
            let mut stmt = cold.connection().prepare(
                "SELECT p.program_id,count(*),\
                        count(DISTINCT printf('%u:%u',e.program_anchor_x,e.program_anchor_y)),\
                        min(e.program_anchor_x),max(e.program_anchor_x),\
                        min(e.program_anchor_y),max(e.program_anchor_y),\
                        min(e.cc_cost),length(p.program),min(e.edge_id),\
                        lower(hex(p.program_sha256)),p.program \
                 FROM programs p JOIN edges e ON e.program_id=p.program_id \
                 GROUP BY p.program_id \
                 ORDER BY count(*) DESC,p.program_id ASC LIMIT ?1",
            )?;
            let rows = stmt.query_map([i64::try_from(limit)?], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Vec<u8>>(11)?,
                ))
            })?;
            for row in rows {
                let (id, uses, anchors, min_x, max_x, min_y, max_y, cc, bytes, example, sha, blob) =
                    row?;
                let template = CheckpointedProofProgram::decode(&blob)?;
                let instructions = bytes.saturating_sub(12) / 8;
                let description = format!("{:?}", template.instructions)
                    .replace('\t', " ")
                    .replace('\n', " ");
                text.push_str(&format!(
                    "{id}\t{uses}\t{anchors}\t{min_x}\t{max_x}\t{min_y}\t{max_y}\t{cc}\t{instructions}\t{example}\t{sha}\t{description}\n"
                ));
            }
            write_text_atomic(&report, &text)?;
            println!("published program-template report: {}", report.display());
        }
        "program-template-occurrences" => {
            let snapshot = args
                .get(1)
                .map(PathBuf::from)
                .ok_or("program-template-occurrences needs a snapshot path")?;
            let template_id: u32 = args
                .get(2)
                .ok_or("program-template-occurrences needs a template id")?
                .parse()?;
            let limit = flag_u64(&args[2..], "--limit", 100)?;
            let cold = SqliteSnapshot::open_file(&snapshot)?;
            if cold.schema_version()? < 3 {
                return Err("program templates require schema version 3".into());
            }
            println!(
                "edge_id\tsource_node\ttarget_node\tanchor_x\tanchor_y\tcc_cost\tfirst_action"
            );
            let mut stmt = cold.connection().prepare(
                "SELECT edge_id,source_node,target_node,program_anchor_x,program_anchor_y,\
                        cc_cost,first_action FROM edges WHERE program_id=?1 \
                 ORDER BY edge_id LIMIT ?2",
            )?;
            let rows = stmt.query_map([i64::from(template_id), i64::try_from(limit)?], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?;
            for row in rows {
                let (edge, source, target, x, y, cc, first) = row?;
                println!("{edge}\t{source}\t{target}\t{x}\t{y}\t{cc}\t{first}");
            }
        }
        "validate" => {
            let path = positional_path(&args)?;
            let started = Instant::now();
            let cold = SqliteSnapshot::open_file(&path)?;
            cold.verify_program_hashes()?;
            let replayed_edges = cold.verify_full_replay()?;
            let snapshot = GraphSnapshot::load_connection(cold.connection())?;
            println!(
                "validated snapshot={} nodes={} edges={} replayed_edges={} elapsed_seconds={:.3}",
                path.display(),
                snapshot.node_count(),
                snapshot.edge_count(),
                replayed_edges,
                started.elapsed().as_secs_f64()
            );
        }
        "inspect" => {
            let path = positional_path(&args)?;
            let started = Instant::now();
            let cold = SqliteSnapshot::open_file(&path)?;
            cold.verify_program_hashes()?;
            let snapshot = GraphSnapshot::load_file(&path)?;
            let bytes = std::fs::metadata(&path)?.len();
            println!("path:                 {}", path.display());
            println!("file bytes:           {bytes}");
            println!("nodes:                {}", snapshot.node_count());
            println!("edges:                {}", snapshot.edge_count());
            println!("schema version:       {}", cold.schema_version()?);
            println!("snapshot version:     {}", snapshot.meta().snapshot_version);
            println!("key spec:             {}", snapshot.meta().key_spec);
            println!(
                "normalizer version:   {}",
                snapshot.meta().normalizer_version
            );
            println!(
                "representation codec: {}",
                snapshot.meta().representation_codec
            );
            println!("action codec:          {}", snapshot.meta().action_codec);
            println!(
                "policy model:         {}",
                snapshot.meta().policy_model_id.as_deref().unwrap_or("none")
            );
            println!(
                "policy objective:     {}",
                snapshot
                    .meta()
                    .policy_objective_ratio
                    .map(|ratio| format!("L{ratio}"))
                    .unwrap_or_else(|| "none".into())
            );
            println!(
                "policy adapter:       {}",
                snapshot
                    .meta()
                    .policy_adapter_version
                    .as_deref()
                    .unwrap_or("none")
            );
            println!(
                "controller start:     {}",
                snapshot
                    .meta()
                    .controller_initial_state
                    .as_deref()
                    .unwrap_or("none")
            );
            println!("synthetic:             {}", snapshot.meta().synthetic);
            println!(
                "validated/load time:  {:.3}s",
                started.elapsed().as_secs_f64()
            );
        }
        "bench" => {
            let path = positional_path(&args)?;
            let query_count = flag_u64(&args, "--queries", 100_000)? as usize;
            let miss_percent = flag_u64(&args, "--miss-percent", 5)?;
            if query_count == 0 {
                return Err("--queries must be at least 1".into());
            }
            if miss_percent > 100 {
                return Err("--miss-percent must be between 0 and 100".into());
            }
            benchmark(&path, query_count, miss_percent as u8)?;
        }
        "bench-population" => {
            let path = positional_path(&args)?;
            let duplicate_rounds = flag_u64(&args, "--duplicate-rounds", 10)? as usize;
            let refresh_rounds = flag_u64(&args, "--refresh-rounds", 10)? as usize;
            let rebuild_rounds = flag_u64(&args, "--rebuild-rounds", 1)? as usize;
            let metrics = PopulationGraph::benchmark_updates(
                &path,
                duplicate_rounds,
                refresh_rounds,
                rebuild_rounds,
            )?;
            println!("snapshot: {}", path.display());
            println!("active unknot edges: {}", metrics.active_unknot_edges);
            println!(
                "validated population load: {:.3} ms",
                metrics.load_time.as_secs_f64() * 1_000.0
            );
            println!(
                "duplicate proof relaxations: {} in {:.3} ms ({:.0}/s, {:.3} us/op)",
                metrics.duplicate_relaxations,
                metrics.duplicate_time.as_secs_f64() * 1_000.0,
                metrics.duplicate_relaxations as f64 / metrics.duplicate_time.as_secs_f64(),
                metrics.duplicate_time.as_secs_f64() * 1_000_000.0
                    / metrics.duplicate_relaxations as f64
            );
            println!(
                "full ACS10 refreshes: {} in {:.3} ms ({:.3} ms/refresh)",
                metrics.refresh_rounds,
                metrics.refresh_time.as_secs_f64() * 1_000.0,
                metrics.refresh_time.as_secs_f64() * 1_000.0 / metrics.refresh_rounds as f64
            );
            println!(
                "monotone rebuild relaxations: {} in {:.3} ms ({:.0}/s, {:.3} us/op; {} rounds)",
                metrics.rebuild_relaxations,
                metrics.rebuild_time.as_secs_f64() * 1_000.0,
                metrics.rebuild_relaxations as f64 / metrics.rebuild_time.as_secs_f64(),
                metrics.rebuild_time.as_secs_f64() * 1_000_000.0
                    / metrics.rebuild_relaxations as f64,
                metrics.rebuild_rounds
            );
            println!(
                "batched rebuild relaxations: {} in {:.3} ms ({:.0}/s, {:.3} us/op; one ACS10 refresh/round)",
                metrics.batched_rebuild_relaxations,
                metrics.batched_rebuild_time.as_secs_f64() * 1_000.0,
                metrics.batched_rebuild_relaxations as f64
                    / metrics.batched_rebuild_time.as_secs_f64(),
                metrics.batched_rebuild_time.as_secs_f64() * 1_000_000.0
                    / metrics.batched_rebuild_relaxations as f64
            );
        }
        other => return Err(format!("unknown command `{other}`\n\n{USAGE}").into()),
    }
    Ok(())
}

fn braid_from_args(args: &[String], command: &str) -> Result<BraidRepresentation> {
    let strands: u16 = args
        .get(1)
        .ok_or_else(|| format!("{command} needs a strand count"))?
        .parse()
        .map_err(|_| "strand count must be an unsigned 16-bit integer")?;
    let word: Vec<i16> = args
        .get(2)
        .ok_or_else(|| format!("{command} needs a comma-separated word"))?
        .split(',')
        .filter(|part| !part.trim().is_empty())
        .map(|part| {
            part.trim()
                .parse::<i16>()
                .map_err(|_| format!("bad braid letter `{}`", part.trim()))
        })
        .collect::<std::result::Result<_, _>>()?;
    let input = BraidRepresentation {
        strands,
        cyclic_band_generators: args.iter().any(|arg| arg == "--cyclic-band"),
        word,
    };
    input.validate()?;
    Ok(input)
}

fn bootstrap_snapshot(
    path: &Path,
    oracle: &mut ExternalPolicyOracle,
    limits: PolicyLimits,
) -> Result<()> {
    if path.exists() {
        return Err(format!("bootstrap snapshot already exists: {}", path.display()).into());
    }
    let unknot = BraidRepresentation {
        strands: 1,
        cyclic_band_generators: false,
        word: vec![],
    }
    .normalize()?;
    let scramble = ProofProgram {
        actions: vec![
            SemanticAction::StabilizePositive,
            SemanticAction::Insert {
                position: 0,
                generator: 1,
                sign: 1,
            },
            SemanticAction::CrossingChange { position: 1 },
        ],
    };
    let scrambled = scramble.replay(&unknot.representation)?.normalize()?;
    if scrambled.representation.strands != 2 || scrambled.representation.word != [1, 1, 1] {
        return Err("bootstrap scramble no longer produces the canonical trefoil".into());
    }
    if !scrambled.representation.is_knot_closure()? {
        return Err("bootstrap scramble did not produce a knot closure".into());
    }
    let traced = trace_policy_edge(&scrambled.representation, oracle, limits)?;
    if traced.target != unknot {
        return Err("bootstrap trefoil policy edge did not return to B1 []".into());
    }
    let program = traced.program.encode()?;
    let first_action = traced.crossing_change.encode_u63()?;
    let mut audit_material = Vec::new();
    audit_material.extend_from_slice(b"UNKNOTDB_BOOTSTRAP_TREFOIL_AUDIT_V1\0");
    audit_material.extend_from_slice(traced.model_id.as_bytes());
    audit_material.extend_from_slice(&scramble.encode()?);
    audit_material.extend_from_slice(&program);
    let trefoil_audit = sha256_digest(&audit_material)?;
    let terminal_audit = sha256_digest(
        [
            b"UNKNOTDB_TERMINAL_B1_AUDIT_V1\0".as_slice(),
            traced.model_id.as_bytes(),
        ]
        .concat()
        .as_slice(),
    )?;
    let mut population = PopulationGraph::from_unknot(terminal_audit)?;
    let unknot_key = population.unknot_key()?;
    if unknot_key != unknot.key {
        return Err("population root does not match canonical B1 []".into());
    }
    let outcome = population.relax_unknot_edge(
        traced.source.clone(),
        PolicyStopAttestation::PreferredCrossingChange {
            action: first_action,
            audit_sha256: trefoil_audit,
        },
        unknot_key,
        traced.program,
        None,
    )?;
    if outcome != (RelaxationOutcome::Inserted { u_upper_bound: 1 }) {
        return Err(format!("unexpected bootstrap relaxation outcome: {outcome:?}").into());
    }
    let (nodes, edges) = population.into_snapshot_records()?;
    let meta = SnapshotMeta::braid_mirror_orbit_v1(
        "bootstrap-unknot-trefoil-scramble-v1",
        traced.model_id,
    );
    write_snapshot_atomic(path, &meta, nodes, edges)?;
    Ok(())
}

fn populate_unknot_frontier(
    path: &Path,
    manifest_path: &Path,
    oracle: &mut ExternalPolicyOracle,
    policy_limits: PolicyLimits,
    scramble_limits: ScrambleLimits,
) -> Result<()> {
    if path.exists() {
        return Err(format!("frontier snapshot already exists: {}", path.display()).into());
    }
    if manifest_path.exists() {
        return Err(format!(
            "frontier manifest already exists: {}",
            manifest_path.display()
        )
        .into());
    }
    scramble_limits.validate()?;
    let unknot = BraidRepresentation {
        strands: 1,
        cyclic_band_generators: false,
        word: vec![],
    }
    .normalize()?;
    let terminal_audit = sha256_digest(
        [
            b"UNKNOTDB_TERMINAL_B1_FRONTIER_AUDIT_V2\0".as_slice(),
            oracle.model_id().as_bytes(),
            POLICY_ADAPTER_VERSION.as_bytes(),
        ]
        .concat()
        .as_slice(),
    )?;
    let mut population = PopulationGraph::from_unknot(terminal_audit)?;
    let batch = expand_frontier_seed(
        &mut population,
        &unknot,
        oracle,
        policy_limits,
        scramble_limits,
    )?;
    let inserted = batch
        .attempts
        .iter()
        .filter(|attempt| matches!(attempt.disposition, FrontierDisposition::Inserted { .. }))
        .count();
    let improved = batch
        .attempts
        .iter()
        .filter(|attempt| matches!(attempt.disposition, FrontierDisposition::Improved { .. }))
        .count();
    let incomplete = batch
        .attempts
        .iter()
        .filter(|attempt| matches!(attempt.disposition, FrontierDisposition::Incomplete(_)))
        .count();
    let accepted_acs10 = batch
        .attempts
        .iter()
        .filter(|attempt| {
            matches!(
                attempt.disposition,
                FrontierDisposition::AcceptedAcs10 { .. }
            )
        })
        .count();
    let manifest = batch.manifest_tsv();
    let model_id = batch.model_id.clone();
    let attempts = batch.attempts.len();
    let (nodes, edges) = population.into_snapshot_records()?;
    let node_count = nodes.len();
    let edge_count = edges.len();
    let meta = SnapshotMeta::braid_mirror_orbit_v1(
        format!(
            "unknot-frontier-d{}-states{}-candidates{}-v0",
            scramble_limits.depth, scramble_limits.max_states, scramble_limits.max_candidates
        ),
        model_id,
    );
    write_snapshot_atomic(path, &meta, nodes, edges)?;
    write_text_atomic(manifest_path, &manifest)?;
    println!("published frontier snapshot: {}", path.display());
    println!("published frontier manifest: {}", manifest_path.display());
    println!(
        "attempts={attempts} inserted={inserted} improved={improved} accepted_acs10={accepted_acs10} incomplete={incomplete} nodes={node_count} edges={edge_count}"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn resume_frontier(
    input: &Path,
    output: &Path,
    manifest_path: &Path,
    oracle: &mut ExternalPolicyOracle,
    policy_limits: PolicyLimits,
    scramble_limits: ScrambleLimits,
    requested_generations: u32,
    requested_seed_rank: Option<u64>,
) -> Result<()> {
    if input == output {
        return Err("resume-frontier input and output snapshots must differ".into());
    }
    if !input.is_file() {
        return Err(format!("frontier parent snapshot is absent: {}", input.display()).into());
    }
    if output.exists() {
        return Err(format!("resumed snapshot already exists: {}", output.display()).into());
    }
    if manifest_path.exists() {
        return Err(format!(
            "frontier run manifest already exists: {}",
            manifest_path.display()
        )
        .into());
    }
    if requested_generations == 0 {
        return Err("--generations must be at least 1".into());
    }
    scramble_limits.validate()?;

    let (mut population, parent_meta) = PopulationGraph::from_snapshot(input)?;
    let parent_model_id = parent_meta
        .policy_model_id
        .as_deref()
        .ok_or("frontier parent has no frozen policy model ID")?;
    if parent_model_id != oracle.model_id() {
        return Err(format!(
            "frontier parent policy model `{parent_model_id}` does not match oracle `{}`",
            oracle.model_id()
        )
        .into());
    }
    let parent_nodes: u64 = population
        .node_count()
        .try_into()
        .map_err(|_| "parent node count does not fit u64")?;
    let parent_edges: u64 = population
        .edge_count()
        .try_into()
        .map_err(|_| "parent edge count does not fit u64")?;
    let start_route_rank = requested_seed_rank
        .or_else(|| population.max_route_rank())
        .ok_or("frontier parent has no ranked population nodes")?;
    let mut seeds = population.seeds_with_route_rank(start_route_rank);
    if seeds.is_empty() {
        return Err(
            format!("frontier parent has no seeds at route rank {start_route_rank}").into(),
        );
    }

    let mut completed_generations = 0_u32;
    let mut run_batches = Vec::new();
    for generation in 0..requested_generations {
        if seeds.is_empty() {
            break;
        }
        let mut inserted_keys = Vec::new();
        for seed in seeds {
            let batch = expand_frontier_seed(
                &mut population,
                &seed.representation,
                oracle,
                policy_limits,
                scramble_limits,
            )?;
            inserted_keys.extend(
                batch
                    .inserted
                    .iter()
                    .map(|representation| representation.key),
            );
            run_batches.push(FrontierRunBatch {
                generation,
                seed_key: seed.representation.key,
                seed_u_upper_bound: seed.u_upper_bound,
                seed_route_rank: seed.route_rank,
                batch,
            });
        }
        completed_generations = completed_generations
            .checked_add(1)
            .ok_or("completed generation count overflow")?;
        inserted_keys.sort_unstable();
        inserted_keys.dedup();
        seeds = inserted_keys
            .into_iter()
            .map(|key| {
                population
                    .seed(&key)
                    .ok_or_else(|| "new frontier seed disappeared from population".into())
            })
            .collect::<Result<Vec<_>>>()?;
    }

    let run_manifest = FrontierRunManifest {
        parent_source_generation: parent_meta.source_generation.clone(),
        parent_nodes,
        parent_edges,
        model_id: oracle.model_id().to_owned(),
        start_route_rank,
        requested_generations,
        completed_generations,
        batches: run_batches,
    };
    let manifest = run_manifest.manifest_tsv()?;
    let attempts: usize = run_manifest
        .batches
        .iter()
        .map(|batch| batch.batch.attempts.len())
        .sum();
    let inserted: usize = run_manifest
        .batches
        .iter()
        .flat_map(|batch| &batch.batch.attempts)
        .filter(|attempt| matches!(attempt.disposition, FrontierDisposition::Inserted { .. }))
        .count();
    let accepted_acs10: usize = run_manifest
        .batches
        .iter()
        .flat_map(|batch| &batch.batch.attempts)
        .filter(|attempt| {
            matches!(
                attempt.disposition,
                FrontierDisposition::AcceptedAcs10 { .. }
            )
        })
        .count();
    let (nodes, edges) = population.into_snapshot_records()?;
    let node_count = nodes.len();
    let edge_count = edges.len();
    let meta = SnapshotMeta::braid_mirror_orbit_v1(
        format!(
            "resume-{}-rank{}-generations{}-v1",
            parent_meta.source_generation, start_route_rank, completed_generations
        ),
        oracle.model_id(),
    );
    write_snapshot_atomic(output, &meta, nodes, edges)?;
    write_text_atomic(manifest_path, &manifest)?;
    println!("published resumed snapshot: {}", output.display());
    println!("published run manifest:     {}", manifest_path.display());
    println!(
        "generations={completed_generations}/{requested_generations} batches={} attempts={attempts} inserted={inserted} accepted_acs10={accepted_acs10} nodes={node_count} edges={edge_count}",
        run_manifest.batches.len()
    );
    Ok(())
}

fn scramble_limits_from_args(args: &[String]) -> Result<ScrambleLimits> {
    Ok(ScrambleLimits {
        depth: flag_u64(args, "--scramble-depth", 3)?
            .try_into()
            .map_err(|_| "--scramble-depth must fit u16")?,
        max_states: flag_u64(args, "--max-scramble-states", 512)?
            .try_into()
            .map_err(|_| "--max-scramble-states must fit u32")?,
        max_candidates: flag_u64(args, "--max-scramble-candidates", 64)?
            .try_into()
            .map_err(|_| "--max-scramble-candidates must fit u32")?,
        max_strands: flag_u64(args, "--max-strands", 3)?
            .try_into()
            .map_err(|_| "--max-strands must fit u16")?,
        max_word_length: flag_u64(args, "--max-word-length", 7)?
            .try_into()
            .map_err(|_| "--max-word-length must fit u32")?,
        min_cc_cost: flag_u64(args, "--min-scramble-cc", 1)?
            .try_into()
            .map_err(|_| "--min-scramble-cc must fit u8")?,
        max_cc_cost: flag_u64(args, "--max-scramble-cc", 1)?
            .try_into()
            .map_err(|_| "--max-scramble-cc must fit u8")?,
    })
}

fn write_text_atomic(path: &Path, contents: &str) -> Result<()> {
    if path.exists() {
        return Err(format!("output already exists: {}", path.display()).into());
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("output path has no UTF-8 file name")?;
    let temporary = parent.join(format!(".{file_name}.{}.part", std::process::id()));
    std::fs::write(&temporary, contents)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

fn ensure_new_outputs(input: &Path, outputs: &[&Path]) -> Result<()> {
    if !input.is_file() {
        return Err(format!("input snapshot is absent: {}", input.display()).into());
    }
    for output in outputs {
        if *output == input {
            return Err("output must differ from input snapshot".into());
        }
        if output.exists() {
            return Err(format!("output already exists: {}", output.display()).into());
        }
    }
    Ok(())
}

fn validate_published_snapshot(path: &Path) -> Result<()> {
    let cold = SqliteSnapshot::open_file(path)?;
    cold.verify_program_hashes()?;
    cold.verify_full_replay()?;
    GraphSnapshot::load_connection(cold.connection())?;
    Ok(())
}

fn sha256_digest(bytes: &[u8]) -> Result<RepKey> {
    let encoded = unknotdb::util::sha256_hex(bytes);
    let mut digest = [0_u8; 32];
    for (index, pair) in encoded.as_bytes().chunks_exact(2).enumerate() {
        digest[index] = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
    }
    Ok(digest)
}

fn benchmark(path: &Path, query_count: usize, miss_percent: u8) -> Result<()> {
    // Load once to draw the same deterministic query set for every backend.
    // This setup load is not included in any reported backend load time.
    let seed_snapshot = GraphSnapshot::load_file(path)?;
    let queries = make_queries(&seed_snapshot, query_count, miss_percent);
    let nodes = seed_snapshot.node_count();
    let edges = seed_snapshot.edge_count();
    drop(seed_snapshot);

    println!(
        "snapshot: {} nodes, {} edges, {:.2} MiB; queries: {} ({}% requested misses)",
        nodes,
        edges,
        std::fs::metadata(path)?.len() as f64 / (1024.0 * 1024.0),
        query_count,
        miss_percent
    );

    let started = Instant::now();
    let file = SqliteSnapshot::open_file(path)?;
    let file_load = started.elapsed();
    let file_rss = current_rss_kib();
    warm_sqlite(&file, &queries)?;
    let file_metrics = measure("sqlite-file/mmap", file_load, file_rss, &queries, |key| {
        file.lookup(key).expect("SQLite file lookup failed")
    });
    drop(file);

    let started = Instant::now();
    let memory = SqliteSnapshot::open_deserialized(path)?;
    let memory_load = started.elapsed();
    let memory_rss = current_rss_kib();
    warm_sqlite(&memory, &queries)?;
    let memory_metrics = measure(
        "sqlite-deserialized",
        memory_load,
        memory_rss,
        &queries,
        |key| memory.lookup(key).expect("SQLite memory lookup failed"),
    );
    drop(memory);

    let started = Instant::now();
    let hash = GraphSnapshot::load_file(path)?;
    let hash_load = started.elapsed();
    let hash_rss = current_rss_kib();
    warm_hash(&hash, &queries);
    let hash_metrics = measure("hashmap+arrays", hash_load, hash_rss, &queries, |key| {
        hash.lookup(key)
    });

    println!(
        "{:<21} {:>10} {:>12} {:>10} {:>10} {:>10} {:>12}",
        "backend", "load ms", "queries/s", "p50 us", "p95 us", "p99 us", "RSS KiB*"
    );
    for metrics in [file_metrics, memory_metrics, hash_metrics] {
        print_metrics(&metrics);
    }
    println!("* current process RSS after load; query buffers and allocator retention are shared");
    Ok(())
}

fn make_queries(snapshot: &GraphSnapshot, count: usize, miss_percent: u8) -> Vec<RepKey> {
    let mut state = 0x4d59_5df4_d0f3_3173_u64;
    let mut queries = Vec::with_capacity(count);
    for i in 0..count {
        state = splitmix64(state);
        let id = (state as usize % snapshot.node_count()) as u32;
        let mut key = *snapshot.key_for_node(id).expect("dense node key");
        state = splitmix64(state);
        if state % 100 < u64::from(miss_percent) {
            // A 256-bit derived key has negligible collision probability. The
            // extra counter avoids repeating one negative lookup in the cache.
            key[0] ^= 0x80;
            key[24..].copy_from_slice(&(i as u64).to_le_bytes());
        }
        queries.push(key);
    }
    queries
}

fn warm_sqlite(snapshot: &SqliteSnapshot, queries: &[RepKey]) -> Result<()> {
    for key in queries.iter().take(queries.len().min(10_000)) {
        black_box(snapshot.lookup(key)?);
    }
    Ok(())
}

fn warm_hash(snapshot: &GraphSnapshot, queries: &[RepKey]) {
    for key in queries.iter().take(queries.len().min(10_000)) {
        black_box(snapshot.lookup(key));
    }
}

fn measure<F>(
    backend: &'static str,
    load: Duration,
    rss: Option<u64>,
    queries: &[RepKey],
    mut lookup: F,
) -> Metrics
where
    F: FnMut(&RepKey) -> Option<HotRoute>,
{
    let total_started = Instant::now();
    let mut latencies = Vec::with_capacity(queries.len());
    let mut hits = 0_u64;
    let mut checksum = 0_u64;
    for key in queries {
        let started = Instant::now();
        let hit = black_box(lookup(black_box(key)));
        latencies.push(started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64);
        if let Some(route) = hit {
            hits += 1;
            checksum = checksum.wrapping_add(u64::from(route.node_id));
            if let Some(step) = route.next_unknot {
                checksum = checksum.wrapping_add(step.first_action);
            }
        }
    }
    let query_total = total_started.elapsed();
    latencies.sort_unstable();
    Metrics {
        backend,
        load,
        query_total,
        p50_ns: percentile(&latencies, 50),
        p95_ns: percentile(&latencies, 95),
        p99_ns: percentile(&latencies, 99),
        query_count: queries.len(),
        hits,
        checksum: black_box(checksum),
        rss_kib_after_load: rss,
    }
}

fn percentile(sorted: &[u64], percentile: usize) -> u64 {
    let index = ((sorted.len() - 1) * percentile) / 100;
    sorted[index]
}

fn print_metrics(metrics: &Metrics) {
    let qps = metrics.query_count as f64 * metrics.query_total.as_secs_f64().recip();
    let rss = metrics
        .rss_kib_after_load
        .map(|v| v.to_string())
        .unwrap_or_else(|| "-".into());
    println!(
        "{:<21} {:>10.3} {:>12.0} {:>10.3} {:>10.3} {:>10.3} {:>12}  hits={} checksum={}",
        metrics.backend,
        metrics.load.as_secs_f64() * 1_000.0,
        qps,
        metrics.p50_ns as f64 / 1_000.0,
        metrics.p95_ns as f64 / 1_000.0,
        metrics.p99_ns as f64 / 1_000.0,
        rss,
        metrics.hits,
        metrics.checksum
    );
}

fn current_rss_kib() -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

fn positional_path(args: &[String]) -> Result<PathBuf> {
    args.iter()
        .skip(1)
        .find(|arg| !arg.starts_with("--") && arg.parse::<u64>().is_err())
        .map(PathBuf::from)
        .ok_or_else(|| "missing snapshot path".into())
}

fn flag_u64(args: &[String], flag: &str, default: u64) -> Result<u64> {
    match args.iter().position(|arg| arg == flag) {
        Some(index) => args
            .get(index + 1)
            .ok_or_else(|| format!("{flag} needs a value").into())
            .and_then(|value| {
                value
                    .parse::<u64>()
                    .map_err(|_| format!("{flag} needs a non-negative integer").into())
            }),
        None => Ok(default),
    }
}

fn optional_flag_u64(args: &[String], flag: &str) -> Result<Option<u64>> {
    args.iter()
        .position(|arg| arg == flag)
        .map(|index| {
            args.get(index + 1)
                .ok_or_else(|| format!("{flag} needs a value").into())
                .and_then(|value| {
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("{flag} needs a non-negative integer").into())
                })
        })
        .transpose()
}

fn flag_path(args: &[String], flag: &str) -> Result<PathBuf> {
    let index = args
        .iter()
        .position(|arg| arg == flag)
        .ok_or_else(|| format!("missing required {flag}"))?;
    args.get(index + 1)
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} needs a path").into())
}

fn flag_string(args: &[String], flag: &str) -> Result<String> {
    optional_flag_string(args, flag)?.ok_or_else(|| format!("missing required {flag}").into())
}

fn optional_flag_string(args: &[String], flag: &str) -> Result<Option<String>> {
    args.iter()
        .position(|arg| arg == flag)
        .map(|index| {
            args.get(index + 1)
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value").into())
        })
        .transpose()
}

fn parse_rep_key(value: &str) -> Result<RepKey> {
    if value.len() != 64 {
        return Err("representation key must contain 64 hex characters".into());
    }
    let mut key = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let pair = std::str::from_utf8(pair)?;
        key[index] = u8::from_str_radix(pair, 16)
            .map_err(|_| "representation key contains non-hex characters")?;
    }
    Ok(key)
}

fn read_rep_keys(path: &Path) -> Result<Vec<RepKey>> {
    let contents = std::fs::read_to_string(path)?;
    let audit_mode = contents.lines().any(|line| line.starts_with("audit\t"));
    let mut keys = Vec::new();
    for (line_index, line) in contents.lines().enumerate() {
        let value = line.trim();
        if value.is_empty() || value.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = value.split('\t').collect();
        if audit_mode {
            if fields.first() == Some(&"audit")
                && fields.len() >= 7
                && fields[1] != "representation_id"
                && fields[5] == "1"
            {
                keys.push(parse_rep_key(fields[4]).map_err(|error| {
                    format!("{} line {}: {}", path.display(), line_index + 1, error)
                })?);
            }
            continue;
        }
        keys.push(
            parse_rep_key(value).map_err(|error| {
                format!("{} line {}: {}", path.display(), line_index + 1, error)
            })?,
        );
    }
    keys.sort_unstable();
    keys.dedup();
    Ok(keys)
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

fn format_word(word: &[i16]) -> String {
    word.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[allow(dead_code)]
fn synthetic_miss_key(id: u64) -> RepKey {
    synthetic_key(id | (1_u64 << 63))
}
