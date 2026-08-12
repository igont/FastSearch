use anyhow::{Context, Result};
use fastembed::{
    Bgem3Embedding, InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use std::{env, fs, path::PathBuf, time::Instant};

fn files(root: &PathBuf) -> Result<TokenizerFiles> {
    Ok(TokenizerFiles {
        tokenizer_file: fs::read(root.join("tokenizer.json"))?,
        config_file: fs::read(root.join("config.json"))?,
        special_tokens_map_file: fs::read(root.join("special_tokens_map.json"))?,
        tokenizer_config_file: fs::read(root.join("tokenizer_config.json"))?,
    })
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let dot: f32 = left.iter().zip(right).map(|(a, b)| a * b).sum();
    let ln: f32 = left.iter().map(|x| x * x).sum::<f32>().sqrt();
    let rn: f32 = right.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (ln * rn)
}

fn main() -> Result<()> {
    let mut args = env::args().skip(1);
    let profile = args.next().context("profile")?;
    let model = PathBuf::from(args.next().context("model root")?);
    if !model.join("onnx/model.onnx").is_file() { anyhow::bail!("B1_NO_PROVIDER_CACHE_MISSING") }
    let query = args.next().context("query")?;
    if query != "semantic navigation optional provider fallback" { anyhow::bail!("B1_FIXED_QUERY_REQUIRED") }
    let docs: Vec<String> = args.map(|file| fs::read_to_string(file)).collect::<std::io::Result<_>>()?;
    if docs.is_empty() { anyhow::bail!("no documents") }
    let documents_len = docs.len();
    let start = Instant::now();
    let vectors = match profile.as_str() {
        "e5" => {
            let onnx = fs::read(model.join("onnx/model.onnx"))?;
            let local = UserDefinedEmbeddingModel::new(onnx, files(&model.join("onnx"))?)
                .with_pooling(Pooling::Mean);
            let mut runtime = TextEmbedding::try_new_from_user_defined(local, InitOptionsUserDefined::default())?;
            let mut input = vec![query]; input.extend(docs.clone());
            runtime.embed(input, Some(1))?
        }
        "bge" => {
            let mut runtime = Bgem3Embedding::try_new_from_path(
                model.join("onnx"), files(&model.join("onnx"))?, InitOptionsUserDefined::default())?;
            let mut input = vec![query]; input.extend(docs.clone());
            runtime.embed(input, Some(1))?.dense
        }
        _ => anyhow::bail!("unsupported profile"),
    };
    if vectors.len() != documents_len + 1 || vectors.iter().any(|v| v.is_empty() || v.iter().any(|x| !x.is_finite())) {
        anyhow::bail!("invalid vectors")
    }
    let query_norm = vectors[0].iter().map(|x| x * x).sum::<f32>().sqrt();
    if !query_norm.is_finite() || query_norm <= 0.0 { anyhow::bail!("B1_INVALID_VECTOR_NORM") }
    let mut ranks: Vec<(usize, f32)> = vectors[1..].iter().enumerate()
        .map(|(i, vector)| (i, cosine(&vectors[0], vector))).collect();
    ranks.sort_by(|left, right| right.1.total_cmp(&left.1).then(left.0.cmp(&right.0)));
    if ranks.first().map(|entry| entry.0) != Some(0) { anyhow::bail!("B1_REQUIRED_HIT_RANK_FAILED") }
    println!("{{\"profile\":\"{}\",\"dimension\":{},\"norm\":{},\"batch_size\":1,\"elapsed_ms\":{},\"rank\":[{}]}}",
        profile, vectors[0].len(), query_norm, start.elapsed().as_millis(),
        ranks.iter().map(|(i, score)| format!("{{\"index\":{},\"score\":{}}}", i, score)).collect::<Vec<_>>().join(","));
    Ok(())
}
