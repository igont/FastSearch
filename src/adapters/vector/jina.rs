use std::{fs, path::Path};

use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::Value,
};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use crate::domain::{Capability, ErrorKind, ExecutionDevice, FastSearchError};

const MAX_LENGTH: usize = 8_192;
const QUERY_TASK: &str = "retrieval.query";
const PASSAGE_TASK: &str = "retrieval.passage";

#[derive(Clone, Copy)]
pub(super) enum JinaTask {
    Query,
    Passage,
}

struct JinaTaskIds {
    query: i64,
    passage: i64,
}

pub(super) struct JinaOnnxRuntime {
    session: Session,
    tokenizer: Tokenizer,
    tasks: JinaTaskIds,
}

impl JinaOnnxRuntime {
    pub(super) fn open(root: &Path, device: ExecutionDevice) -> Result<Self, FastSearchError> {
        let repository =
            hf_hub::Cache::new(root.to_path_buf()).model("jinaai/jina-embeddings-v3".to_owned());
        let required = |name: &str| {
            repository
                .get(name)
                .ok_or_else(|| jina_error(format!("Jina catalog model is missing {name}")))
        };
        let model_path = required("onnx/model.onnx")?;
        let config_path = required("config.json")?;
        let tokenizer_path = required("tokenizer.json")?;
        let tokenizer_config_path = required("tokenizer_config.json")?;
        let config = fs::read(&config_path).map_err(jina_error)?;
        let tokenizer_config = fs::read(&tokenizer_config_path).map_err(jina_error)?;
        let tasks = task_ids(&config)?;
        let (pad_id, pad_token) = tokenizer_padding(&config, &tokenizer_config)?;
        let mut tokenizer = Tokenizer::from_file(tokenizer_path).map_err(jina_error)?;
        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            pad_id,
            pad_token,
            ..Default::default()
        }));
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_LENGTH,
                ..Default::default()
            }))
            .map_err(jina_error)?;

        let threads = std::thread::available_parallelism()
            .map_err(jina_error)?
            .get();
        let mut builder = Session::builder()
            .map_err(jina_error)?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(jina_error)?
            .with_intra_threads(threads)
            .map_err(jina_error)?;
        if device == ExecutionDevice::GpuDirectMl {
            builder = builder
                .with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()])
                .map_err(jina_error)?
                .with_memory_pattern(false)
                .map_err(jina_error)?
                .with_parallel_execution(false)
                .map_err(jina_error)?;
        }
        let session = builder.commit_from_file(model_path).map_err(jina_error)?;
        let input_names = session
            .inputs()
            .iter()
            .map(|input| input.name())
            .collect::<Vec<_>>();
        if !["input_ids", "attention_mask", "task_id"]
            .iter()
            .all(|expected| input_names.contains(expected))
        {
            return Err(jina_error(format!(
                "Jina ONNX input contract is incompatible: {}",
                input_names.join(", ")
            )));
        }

        Ok(Self {
            session,
            tokenizer,
            tasks,
        })
    }

    pub(super) fn embed(
        &mut self,
        texts: &[String],
        task: JinaTask,
        batch_size: usize,
    ) -> Result<Vec<Vec<f32>>, FastSearchError> {
        if batch_size == 0 {
            return Err(jina_error("Jina batch size must be greater than zero"));
        }
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(batch_size) {
            let encodings = self
                .tokenizer
                .encode_batch(batch.to_vec(), true)
                .map_err(jina_error)?;
            let sequence_length = encodings
                .first()
                .ok_or_else(|| jina_error("Jina tokenizer returned no encodings"))?
                .len();
            let mut input_ids = Vec::with_capacity(batch.len() * sequence_length);
            let mut attention_mask = Vec::with_capacity(batch.len() * sequence_length);
            for encoding in &encodings {
                if encoding.len() != sequence_length {
                    return Err(jina_error("Jina tokenizer did not pad a complete batch"));
                }
                input_ids.extend(encoding.get_ids().iter().map(|value| i64::from(*value)));
                attention_mask.extend(
                    encoding
                        .get_attention_mask()
                        .iter()
                        .map(|value| i64::from(*value)),
                );
            }
            let task_id = match task {
                JinaTask::Query => self.tasks.query,
                JinaTask::Passage => self.tasks.passage,
            };
            let outputs = self
                .session
                .run(ort::inputs![
                    "input_ids" => Value::from_array(([batch.len(), sequence_length], input_ids)).map_err(jina_error)?,
                    "attention_mask" => Value::from_array(([batch.len(), sequence_length], attention_mask.clone())).map_err(jina_error)?,
                    "task_id" => Value::from_array(([1_usize], vec![task_id])).map_err(jina_error)?,
                ])
                .map_err(jina_error)?;
            let token_embeddings = outputs
                .get("text_embeds")
                .ok_or_else(|| jina_error("Jina ONNX output text_embeds is absent"))?;
            let (shape, values) = token_embeddings
                .try_extract_tensor::<f32>()
                .map_err(jina_error)?;
            vectors.extend(mean_pool_and_normalize(
                shape,
                values,
                &attention_mask,
                batch.len(),
                sequence_length,
            )?);
        }
        Ok(vectors)
    }
}

fn task_ids(config: &[u8]) -> Result<JinaTaskIds, FastSearchError> {
    let config: serde_json::Value = serde_json::from_slice(config).map_err(jina_error)?;
    let adaptations = config["lora_adaptations"]
        .as_array()
        .ok_or_else(|| jina_error("Jina config is missing lora_adaptations"))?;
    let find = |name: &str| {
        adaptations
            .iter()
            .position(|value| value.as_str() == Some(name))
            .and_then(|index| i64::try_from(index).ok())
            .ok_or_else(|| jina_error(format!("Jina config is missing {name} task")))
    };
    Ok(JinaTaskIds {
        query: find(QUERY_TASK)?,
        passage: find(PASSAGE_TASK)?,
    })
}

fn tokenizer_padding(
    config: &[u8],
    tokenizer_config: &[u8],
) -> Result<(u32, String), FastSearchError> {
    let config: serde_json::Value = serde_json::from_slice(config).map_err(jina_error)?;
    let tokenizer_config: serde_json::Value =
        serde_json::from_slice(tokenizer_config).map_err(jina_error)?;
    let pad_id = config["pad_token_id"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| jina_error("Jina config is missing pad_token_id"))?;
    let pad_token = tokenizer_config["pad_token"]
        .as_str()
        .ok_or_else(|| jina_error("Jina tokenizer config is missing pad_token"))?
        .to_owned();
    Ok((pad_id, pad_token))
}

fn mean_pool_and_normalize(
    shape: &[i64],
    values: &[f32],
    attention_mask: &[i64],
    batch_size: usize,
    sequence_length: usize,
) -> Result<Vec<Vec<f32>>, FastSearchError> {
    if shape.len() != 3
        || usize::try_from(shape[0]).ok() != Some(batch_size)
        || usize::try_from(shape[1]).ok() != Some(sequence_length)
    {
        return Err(jina_error(format!(
            "Jina text_embeds has incompatible shape {shape:?}"
        )));
    }
    let dimensions = usize::try_from(shape[2])
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| jina_error("Jina text_embeds has invalid dimensions"))?;
    if values.len() != batch_size * sequence_length * dimensions
        || attention_mask.len() != batch_size * sequence_length
    {
        return Err(jina_error(
            "Jina output or attention mask has invalid length",
        ));
    }
    let mut vectors = Vec::with_capacity(batch_size);
    for batch in 0..batch_size {
        let mut vector = vec![0.0_f32; dimensions];
        let mut tokens = 0_u32;
        for token in 0..sequence_length {
            if attention_mask[batch * sequence_length + token] == 0 {
                continue;
            }
            tokens += 1;
            let start = (batch * sequence_length + token) * dimensions;
            for (target, value) in vector.iter_mut().zip(&values[start..start + dimensions]) {
                *target += value;
            }
        }
        if tokens == 0 {
            return Err(jina_error("Jina attention mask contains an empty item"));
        }
        let divisor = tokens as f32;
        for value in &mut vector {
            *value /= divisor;
        }
        let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
        if !norm.is_finite() || norm <= 0.0 {
            return Err(jina_error("Jina returned a non-finite or zero embedding"));
        }
        for value in &mut vector {
            *value /= norm;
        }
        vectors.push(vector);
    }
    Ok(vectors)
}

fn jina_error(error: impl std::fmt::Display) -> FastSearchError {
    FastSearchError::new(
        ErrorKind::CapabilityUnavailable {
            capability: Capability::VectorRetrieval,
        },
        error.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_ids_follow_the_pinned_config_instead_of_a_magic_number() {
        let ids = task_ids(
            br#"{"lora_adaptations":["retrieval.query","retrieval.passage","text-matching"]}"#,
        )
        .unwrap();
        assert_eq!(ids.query, 0);
        assert_eq!(ids.passage, 1);
    }

    #[test]
    fn mean_pooling_ignores_padding_and_returns_unit_vectors() {
        let vectors = mean_pool_and_normalize(
            &[1, 3, 2],
            &[1.0, 0.0, 0.0, 1.0, 99.0, 99.0],
            &[1, 1, 0],
            1,
            3,
        )
        .unwrap();
        let expected = 1.0_f32 / 2.0_f32.sqrt();
        assert!((vectors[0][0] - expected).abs() < 1e-6);
        assert!((vectors[0][1] - expected).abs() < 1e-6);
    }
}
