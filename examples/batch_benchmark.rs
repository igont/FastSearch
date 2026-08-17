use std::{
    env, fs,
    io::{self, IsTerminal},
    path::{Path, PathBuf},
    process::ExitCode,
};

use fastsearch::{
    adapters::vector::benchmark_embedding_batches_on_device,
    application::ensure_embedding_model,
    domain::{EmbeddingModelId, ExecutionDevice},
};
use ignore::WalkBuilder;
use terminal_dialogue::{
    LanguagePack, TableColumn, TableDocument, TableRow, TerminalDocument, write_document,
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let document = terminal_dialogue::UserErrorDocument::new(error.to_string())
                .with_code("BATCH_BENCHMARK_FAILED")
                .to_dialogue_document(&LanguagePack::russian());
            let _ = write_document(
                &mut io::stderr().lock(),
                &document,
                io::stderr().is_terminal(),
            );
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(env::current_dir()?);
    let device = match env::args().nth(2).as_deref() {
        Some("gpu") => ExecutionDevice::GpuDirectMl,
        Some("cpu") | None => ExecutionDevice::Cpu,
        Some(value) => {
            return Err(
                format!("неизвестное устройство `{value}`; используйте cpu или gpu").into(),
            );
        }
    };
    let sample_limit = env::var("FASTSEARCH_BENCHMARK_SAMPLE_SIZE")
        .ok()
        .map(|value| value.parse::<usize>())
        .transpose()?
        .unwrap_or(128);
    let texts = sample_texts(&root, sample_limit)?;
    if texts.len() < 16 {
        return Err(format!(
            "для устойчивого измерения нужно минимум 16 текстовых файлов; найдено {}",
            texts.len()
        )
        .into());
    }

    let model = env::args()
        .nth(4)
        .and_then(|value| EmbeddingModelId::parse(&value))
        .unwrap_or(EmbeddingModelId::MultilingualE5Small);
    let availability = ensure_embedding_model(model, false)?;
    let requested_batch_sizes = env::args()
        .nth(3)
        .map(|value| {
            value
                .split(',')
                .map(str::parse::<usize>)
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?;
    let batch_sizes: &[usize] = if let Some(requested) = requested_batch_sizes.as_deref() {
        requested
    } else if device == ExecutionDevice::GpuDirectMl {
        &[1, 2, 4, 8, 16]
    } else {
        &[1, 2, 4, 8, 16, 32, 64]
    };
    let measurements = benchmark_embedding_batches_on_device(
        model,
        availability.root(),
        &texts,
        batch_sizes,
        device,
    )?;
    let best = measurements
        .iter()
        .max_by(|left, right| {
            left.documents_per_second()
                .total_cmp(&right.documents_per_second())
        })
        .map(|measurement| measurement.batch_size())
        .unwrap_or(1);

    let document = measurements.iter().fold(
        TableDocument::new(
            "Эксперимент batch size",
            vec![
                TableColumn::new("BATCH").right_aligned(),
                TableColumn::new("ВРЕМЯ, МС").right_aligned(),
                TableColumn::new("ДОК/С").right_aligned(),
                TableColumn::new("ПАМЯТЬ").right_aligned(),
                TableColumn::new("ВЫБОР"),
            ],
        )
        .with_summary(format!(
            "{} · {} · {} текстов из {} · медиана трёх запусков",
            model.display_name(),
            device.label(),
            texts.len(),
            root.display()
        )),
        |document, measurement| {
            document.with_row(TableRow::new([
                measurement.batch_size().to_string(),
                measurement.duration_ms().to_string(),
                format!("{:.2}", measurement.documents_per_second()),
                measurement.working_set_bytes().map_or_else(
                    || "—".to_owned(),
                    |bytes| format!("{:.2} ГБ", bytes as f64 / 1_073_741_824.0),
                ),
                if measurement.batch_size() == best {
                    "ЛУЧШИЙ"
                } else {
                    ""
                }
                .to_owned(),
            ]))
        },
    );
    write_document(
        &mut io::stdout().lock(),
        &document.to_dialogue_document(&LanguagePack::russian()),
        io::stdout().is_terminal(),
    )?;
    Ok(())
}

fn sample_texts(root: &Path, limit: usize) -> Result<Vec<String>, io::Error> {
    let mut texts = Vec::with_capacity(limit);
    for entry in WalkBuilder::new(root).hidden(false).build() {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if !entry.file_type().is_some_and(|kind| kind.is_file()) || !is_text_source(path) {
            continue;
        }
        let Ok(source) = fs::read_to_string(path) else {
            continue;
        };
        let source = source.trim();
        if source.is_empty() {
            continue;
        }
        texts.push(source.chars().take(4_000).collect());
        if texts.len() == limit {
            break;
        }
    }
    Ok(texts)
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
