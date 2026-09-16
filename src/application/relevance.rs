//! Admission of reranked passages. Scores are never presented as calibrated probabilities.

use super::PublicSearchError;

// Training-only midpoint, independently checked on held-out CadFrame passages.
// This is an admission cutoff, NOT a calibrated probability. Changing the pinned
// model, prompt or token budget requires re-scoring and re-evaluating the corpus.
const MIN_RELEVANCE_SCORE: f32 = 0.887_992_1;

pub(super) fn admits(score: f32) -> Result<bool, PublicSearchError> {
    if !score.is_finite() || !(0.0..=1.0).contains(&score) {
        return Err(PublicSearchError::search_failed("invalid reranker score"));
    }
    Ok(score >= MIN_RELEVANCE_SCORE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::qwen_reranker::{QWEN_INSTRUCTION, QWEN_MAX_TOKENS, QWEN_REVISION};

    #[test]
    fn cutoff_matches_accepted_evaluation_and_exact_scoring_contract() {
        let report: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/relevance/evaluation.json"
        ))
        .unwrap();
        assert_eq!(report["accepted"], true);
        assert_eq!(
            report["threshold"].as_f64().unwrap() as f32,
            MIN_RELEVANCE_SCORE
        );
        assert_eq!(report["model_revision"], QWEN_REVISION);
        assert_eq!(report["instruction"], QWEN_INSTRUCTION);
        assert_eq!(report["max_tokens"], QWEN_MAX_TOKENS);
    }

    #[test]
    fn held_out_answers_survive_and_unrelated_passages_are_rejected() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/relevance/cases.json"))
                .unwrap();
        let scores: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/relevance/scores.json"))
                .unwrap();
        for pair in cases["pairs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|pair| pair["split"] == "validation")
        {
            let score = scores["scores"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == pair["id"])
                .unwrap()["score"]
                .as_f64()
                .unwrap() as f32;
            assert_eq!(
                admits(score).unwrap(),
                pair["relevant"].as_bool().unwrap(),
                "{}",
                pair["id"]
            );
        }
    }

    #[test]
    fn rank_does_not_admit_a_weak_best_result() {
        assert!(!admits(0.01).unwrap());
        assert!(!admits(0.0).unwrap());
        assert!(admits(1.0).unwrap());
        assert!(!admits(MIN_RELEVANCE_SCORE - 0.00001).unwrap());
        assert!(admits(MIN_RELEVANCE_SCORE).unwrap());
        for invalid in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(admits(invalid).is_err());
        }
    }
}
