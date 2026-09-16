//! Score a frozen, labelled corpus with the exact production reranker.
use std::{env, fs, path::Path};

use fastsearch::adapters::qwen_reranker::{
    QWEN_INSTRUCTION, QWEN_MAX_TOKENS, QWEN_REVISION, QwenReranker,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let [cases, model, manifest, output] = args.as_slice() else {
        return Err(
            "usage: score_relevance <cases.json> <model-root> <manifest.json> <output.json>".into(),
        );
    };
    let bytes = fs::read(cases)?;
    let fixture: Value = serde_json::from_slice(&bytes)?;
    let pairs = fixture["pairs"].as_array().ok_or("missing pairs")?;
    let mut reranker = QwenReranker::open(Path::new(model), Path::new(manifest))?;
    let mut scores = Vec::new();
    for (index, pair) in pairs.iter().enumerate() {
        let query = pair["query"].as_str().ok_or("missing query")?;
        let document = pair["document"].as_str().ok_or("missing document")?;
        let score = reranker.score(query, document)?;
        eprintln!("{}/{} {}: {score:.6}", index + 1, pairs.len(), pair["id"]);
        scores.push(json!({"id": pair["id"], "score": score}));
    }
    fs::write(
        output,
        serde_json::to_vec_pretty(&json!({
            "schema": 1,
            "model_revision": QWEN_REVISION,
            "instruction": QWEN_INSTRUCTION,
            "max_tokens": QWEN_MAX_TOKENS,
            "cases_sha256": format!("{:x}", Sha256::digest(&bytes)),
            "scores": scores
        }))?,
    )?;
    Ok(())
}
