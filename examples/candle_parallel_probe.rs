use std::{
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};

use fastsearch::{
    adapters::vector::LocalE5Vector,
    application::ensure_embedding_model,
    domain::{CanonicalRecord, ContentHash, EmbeddingModelId, RecordKind, SourceLocator, StableId},
};
use ignore::WalkBuilder;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("CANDLE_PARALLEL_PROBE_FAILED: {error}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    let model = env::args()
        .nth(2)
        .and_then(|value| EmbeddingModelId::parse(&value))
        .unwrap_or(EmbeddingModelId::Qwen3Embedding06B);
    let sample_limit = env::var("FASTSEARCH_PARALLEL_PROBE_SAMPLE_SIZE")
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(16);
    let records = sample_records(&root, sample_limit)?;
    if records.len() < 16 {
        return Err(format!("need at least 16 text files; found {}", records.len()).into());
    }
    let availability = ensure_embedding_model(model, false)?;
    let vector = LocalE5Vector::open_with_model(
        availability.root(),
        format!("{}@parallel-probe", model.slug()),
        model,
    );
    let started = Instant::now();
    let status = vector.apply(&records, 1)?;
    println!(
        "model={} records={} elapsed_ms={} freshness={:?}",
        model.slug(),
        records.len(),
        started.elapsed().as_millis(),
        status.freshness()
    );
    Ok(())
}

fn sample_records(root: &Path, limit: usize) -> Result<Vec<CanonicalRecord>, std::io::Error> {
    let mut records = Vec::with_capacity(limit);
    for entry in WalkBuilder::new(root).hidden(false).build() {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !entry.file_type().is_some_and(|kind| kind.is_file()) || !is_text_source(path) {
            continue;
        }
        let Ok(content) = fs::read_to_string(path) else {
            continue;
        };
        let content = content.trim();
        if content.is_empty() {
            continue;
        }
        let index = records.len();
        records.push(
            CanonicalRecord::new(
                StableId::parse(format!("parallel-probe-{index}"))
                    .expect("probe identifier is valid"),
                RecordKind::MarkdownSection,
                SourceLocator::markdown(format!("probe-{index}.md"), ["parallel-probe"])
                    .expect("probe locator is valid"),
                format!("parallel probe {index}"),
                content.chars().take(4_000).collect::<String>(),
                Default::default(),
                Vec::new(),
                ContentHash::parse(format!("parallel-probe-{index}"))
                    .expect("probe content hash is valid"),
            )
            .expect("probe record is valid"),
        );
        if records.len() == limit {
            break;
        }
    }
    Ok(records)
}

fn is_text_source(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "md" | "rs" | "toml" | "txt" | "tsv" | "py" | "json"
            )
        })
}
