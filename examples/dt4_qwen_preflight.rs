use std::{collections::BTreeMap, fs, path::PathBuf, time::Instant};

use fastsearch::adapters::qwen_reranker::{
    QWEN_NO_TOKEN, QWEN_REVISION, QWEN_YES_TOKEN, QwenReranker,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Fixture {
    pairs: Vec<Pair>,
}

#[derive(Deserialize)]
struct Pair {
    id: String,
    query: String,
    document: String,
}

#[derive(Deserialize)]
struct Oracle {
    scores: Vec<Score>,
    order: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
struct Score {
    id: String,
    probability_yes: f32,
}

#[derive(Serialize)]
struct Evidence {
    schema: u32,
    gate: &'static str,
    status: &'static str,
    model_revision: &'static str,
    yes_token: u32,
    no_token: u32,
    tolerance: f32,
    free_physical_memory_before_bytes: Option<u64>,
    free_physical_memory_after_bytes: Option<u64>,
    working_set_after_open_bytes: Option<u64>,
    working_set_after_inference_bytes: Option<u64>,
    cold_open_ms: u128,
    inference_ms: u128,
    files: BTreeMap<String, String>,
    cases_sha256: String,
    oracle_sha256: String,
    rust_scores: Vec<Score>,
    python_order: Vec<String>,
    rust_order: Vec<String>,
    maximum_absolute_delta: f32,
}

fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let model_root = PathBuf::from(std::env::args_os().nth(1).ok_or("model root required")?);
    let cases_path = PathBuf::from(std::env::args_os().nth(2).ok_or("cases path required")?);
    let oracle_path = PathBuf::from(std::env::args_os().nth(3).ok_or("oracle path required")?);
    let output_path = PathBuf::from(std::env::args_os().nth(4).ok_or("output path required")?);
    let fixture: Fixture = serde_json::from_slice(&fs::read(&cases_path)?)?;
    let oracle: Oracle = serde_json::from_slice(&fs::read(&oracle_path)?)?;
    let expected: BTreeMap<_, _> = oracle
        .scores
        .iter()
        .map(|score| (score.id.as_str(), score.probability_yes))
        .collect();

    let free_physical_memory_before_bytes = free_physical_memory_bytes();
    let open_started = Instant::now();
    let mut model = QwenReranker::open(&model_root)?;
    let cold_open_ms = open_started.elapsed().as_millis();
    let working_set_after_open_bytes = working_set_bytes();
    let inference_started = Instant::now();
    let mut rust_scores = Vec::with_capacity(fixture.pairs.len());
    let mut maximum_absolute_delta = 0.0_f32;
    for pair in fixture.pairs {
        let probability_yes = model.score(&pair.query, &pair.document)?;
        let delta = (probability_yes - expected[&pair.id.as_str()]).abs();
        maximum_absolute_delta = maximum_absolute_delta.max(delta);
        rust_scores.push(Score {
            id: pair.id,
            probability_yes,
        });
    }
    let inference_ms = inference_started.elapsed().as_millis();
    let working_set_after_inference_bytes = working_set_bytes();
    let mut ordered = rust_scores.clone();
    ordered.sort_by(|left, right| {
        right
            .probability_yes
            .total_cmp(&left.probability_yes)
            .then_with(|| left.id.cmp(&right.id))
    });
    let rust_order = ordered
        .into_iter()
        .map(|score| score.id)
        .collect::<Vec<_>>();
    if maximum_absolute_delta > 0.02 || rust_order != oracle.order {
        return Err(format!(
            "Qwen oracle mismatch: max delta={maximum_absolute_delta}, rust order={rust_order:?}, python order={:?}",
            oracle.order
        )
        .into());
    }
    let files = ["config.json", "tokenizer.json", "model.safetensors"]
        .into_iter()
        .map(|name| Ok((name.to_owned(), sha256(&model_root.join(name))?)))
        .collect::<Result<BTreeMap<_, _>, std::io::Error>>()?;
    let evidence = Evidence {
        schema: 1,
        gate: "G-QWEN",
        status: "PASS",
        model_revision: QWEN_REVISION,
        yes_token: QWEN_YES_TOKEN,
        no_token: QWEN_NO_TOKEN,
        tolerance: 0.02,
        free_physical_memory_before_bytes,
        free_physical_memory_after_bytes: free_physical_memory_bytes(),
        working_set_after_open_bytes,
        working_set_after_inference_bytes,
        cold_open_ms,
        inference_ms,
        files,
        cases_sha256: sha256(&cases_path)?,
        oracle_sha256: sha256(&oracle_path)?,
        rust_scores,
        python_order: oracle.order,
        rust_order,
        maximum_absolute_delta,
    };
    fs::write(output_path, serde_json::to_vec_pretty(&evidence)?)?;
    Ok(())
}

#[cfg(windows)]
fn free_physical_memory_bytes() -> Option<u64> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status = MEMORYSTATUSEX {
        dwLength: u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).ok()?,
        ..MEMORYSTATUSEX::default()
    };
    // SAFETY: status is a correctly sized writable structure for this call.
    (unsafe { GlobalMemoryStatusEx(&raw mut status) } != 0).then_some(status.ullAvailPhys)
}

#[cfg(windows)]
fn working_set_bytes() -> Option<u64> {
    use windows_sys::Win32::System::{
        ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        Threading::GetCurrentProcess,
    };
    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: u32::try_from(std::mem::size_of::<PROCESS_MEMORY_COUNTERS>()).ok()?,
        ..PROCESS_MEMORY_COUNTERS::default()
    };
    // SAFETY: the pseudo-handle is valid and counters is correctly sized.
    (unsafe { GetProcessMemoryInfo(GetCurrentProcess(), (&raw mut counters).cast(), counters.cb) }
        != 0)
        .then_some(counters.WorkingSetSize as u64)
}

#[cfg(not(windows))]
fn free_physical_memory_bytes() -> Option<u64> {
    None
}

#[cfg(not(windows))]
fn working_set_bytes() -> Option<u64> {
    None
}

fn sha256(path: &std::path::Path) -> Result<String, std::io::Error> {
    let bytes = fs::read(path)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
