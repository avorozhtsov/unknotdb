//! Mandatory deterministic decreasing reducer used before and after policy work.

use crate::representation::{
    BraidRepresentation, NormalizationWitness, NormalizedRepresentation, ProofProgram,
    SemanticAction,
};
use crate::Result;

pub const REDUCER_VERSION: &str = "l10-ri-r2-decreasing-v0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReductionReport {
    pub input: BraidRepresentation,
    pub input_normalization: NormalizationWitness,
    pub source: NormalizedRepresentation,
    pub program: ProofProgram,
    pub output: NormalizedRepresentation,
    pub l10_before: u64,
    pub l10_after: u64,
}

impl ReductionReport {
    /// Replay both the initial origin witness and every decreasing primitive.
    pub fn verify(&self) -> Result<()> {
        let canonical_source = self.input.normalize()?;
        if canonical_source != self.source || canonical_source.witness != self.input_normalization {
            return Err("reducer source is not the canonical normalization of input".into());
        }
        let normalized_input = self.input_normalization.apply(&self.input)?;
        if normalized_input != self.source.representation {
            return Err("reducer input-normalization witness does not reproduce source".into());
        }
        if self.l10_before != complexity_l10(&self.source.representation) {
            return Err("reducer l10_before does not match source".into());
        }
        let mut state = self.source.representation.clone();
        for (index, action) in self.program.actions.iter().copied().enumerate() {
            if !matches!(
                action,
                SemanticAction::Destabilize | SemanticAction::Reduce { .. }
            ) {
                return Err(format!("reducer action {} is not decreasing RI/RII", index).into());
            }
            let before = complexity_l10(&state);
            state = action
                .apply(&state)
                .map_err(|error| format!("reducer action {} {:?}: {}", index, action, error))?;
            let after = complexity_l10(&state);
            if after >= before {
                return Err(format!(
                    "reducer action {} does not decrease L10: {} -> {}",
                    index, before, after
                )
                .into());
            }
        }
        if self.program.cc_cost() != 0 {
            return Err("mandatory reducer contains a crossing change".into());
        }
        if SemanticAction::Destabilize.apply(&state).is_ok()
            || first_cyclic_reduction(&state).is_some()
        {
            return Err("reducer output is not an RI/RII fixed point".into());
        }
        let output = state.normalize()?;
        if output != self.output {
            return Err("reducer replay does not reproduce normalized output".into());
        }
        if self.l10_after != complexity_l10(&self.output.representation) {
            return Err("reducer l10_after does not match output".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DeterministicReducer;

impl DeterministicReducer {
    pub fn reduce(input: &BraidRepresentation) -> Result<ReductionReport> {
        let source = input.normalize()?;
        let mut state = source.representation.clone();
        let l10_before = complexity_l10(&state);
        let mut actions = Vec::new();

        loop {
            let action = if SemanticAction::Destabilize.apply(&state).is_ok() {
                Some(SemanticAction::Destabilize)
            } else {
                first_cyclic_reduction(&state)
            };
            let Some(action) = action else {
                break;
            };
            let before = complexity_l10(&state);
            let next = action.apply(&state)?;
            let after = complexity_l10(&next);
            if after >= before {
                return Err(format!(
                    "internal reducer bug: {:?} changed L10 {} -> {}",
                    action, before, after
                )
                .into());
            }
            actions.push(action);
            state = next;
        }

        let output = state.normalize()?;
        let report = ReductionReport {
            input: input.clone(),
            input_normalization: source.witness,
            l10_before,
            l10_after: complexity_l10(&output.representation),
            source,
            program: ProofProgram { actions },
            output,
        };
        report.verify()?;
        Ok(report)
    }
}

/// `L10` is the representation-only complexity used by the mandatory reducer.
/// It deliberately excludes the graph's unknotting upper bound.
pub fn complexity_l10(representation: &BraidRepresentation) -> u64 {
    10 * u64::from(representation.strands) + representation.word.len() as u64
}

fn first_cyclic_reduction(state: &BraidRepresentation) -> Option<SemanticAction> {
    let len = state.word.len();
    if len < 2 {
        return None;
    }
    (0..len)
        .find(|&position| state.word[position] == -state.word[(position + 1) % len])
        .map(|position| SemanticAction::Reduce {
            position: position as u32,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn braid(strands: u16, word: &[i16]) -> BraidRepresentation {
        BraidRepresentation {
            strands,
            cyclic_band_generators: false,
            word: word.to_vec(),
        }
    }

    #[test]
    fn destabilization_has_priority_over_reduction_and_every_step_decreases_l10() {
        let report = DeterministicReducer::reduce(&braid(3, &[2, 1, -1])).unwrap();
        assert_eq!(
            report.program.actions,
            vec![
                SemanticAction::Destabilize,
                SemanticAction::Reduce { position: 0 }
            ]
        );
        assert_eq!(report.l10_before, 33);
        assert_eq!(report.l10_after, 20);
        assert_eq!(report.output.representation, braid(2, &[]));
        report.verify().unwrap();
    }

    #[test]
    fn cyclic_r2_across_the_seam_is_found() {
        let report = DeterministicReducer::reduce(&braid(4, &[1, 2, -1])).unwrap();
        assert_eq!(
            report.program.actions,
            vec![SemanticAction::Reduce { position: 0 }]
        );
        assert_eq!(report.output.representation, braid(4, &[2]));
    }

    #[test]
    fn reducer_is_idempotent() {
        let first = DeterministicReducer::reduce(&braid(4, &[1, 2, -1])).unwrap();
        let second = DeterministicReducer::reduce(&first.output.representation).unwrap();
        assert!(second.program.actions.is_empty());
        assert_eq!(second.output.key, first.output.key);
        assert_eq!(second.l10_before, second.l10_after);
    }

    #[test]
    fn verifier_rejects_a_truncated_non_maximal_reduction() {
        let mut report = DeterministicReducer::reduce(&braid(4, &[1, -1])).unwrap();
        report.program.actions.clear();
        report.output = report.source.clone();
        report.l10_after = report.l10_before;
        assert!(report.verify().is_err());
    }

    #[test]
    fn bstar_seam_blocks_markov_destabilization_but_not_r2() {
        let input = BraidRepresentation {
            strands: 4,
            cyclic_band_generators: true,
            word: vec![3, 4, -4],
        };
        let report = DeterministicReducer::reduce(&input).unwrap();
        assert_eq!(
            report.program.actions,
            vec![
                SemanticAction::Reduce { position: 2 },
                SemanticAction::Destabilize
            ]
        );
        assert_eq!(
            report.output.representation,
            BraidRepresentation {
                strands: 3,
                cyclic_band_generators: true,
                word: vec![],
            }
        );
    }
}
