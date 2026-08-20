//! Explicit multi-model experiment contour.
//!
//! Normal search remains bound to one workspace model.  This coordinator
//! reuses the shared canonical/lexical state, owns no rendering, and never
//! changes the workspace model selection.

use std::{sync::mpsc, thread, time::Instant};

use crate::{
    adapters::vector::VectorBuildProgress,
    domain::{
        EmbeddingModelId, FastSearchError, IndexFreshness, LifecycleStatus, SearchHit, SearchQuery,
    },
    ports::AgentSurface,
};

use super::{
    ModelPartitionMetrics, ModelProvisionProgress, ProductionRuntime, embedding_model_cache_status,
    ensure_embedding_model_with_progress,
    model_cache::download_embedding_model_assets_with_progress,
    production::{IndexingProgress, IndexingStage},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ComparisonSharedStage {
    Sources,
    State,
    Lexical,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ComparisonModelStage {
    Checking,
    Downloading {
        asset: Option<String>,
        completed_bytes: Option<u64>,
        total_bytes: Option<u64>,
    },
    /// Weights are available; one shared lane will validate and index this
    /// model after the model currently being materialized.
    QueuedForIndexing,
    Validating,
    PreparingForIndexing,
    Indexing {
        completed_records: u64,
        total_records: u64,
    },
    Saving,
    Completed {
        reused: bool,
    },
    Failed {
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ComparisonUpdateProgress {
    Shared {
        completed: u64,
        total: u64,
        stage: ComparisonSharedStage,
    },
    SharedCompleted,
    SharedFailed {
        message: String,
    },
    Model {
        model: EmbeddingModelId,
        stage: ComparisonModelStage,
    },
}

enum DownloadMessage {
    Progress {
        model: EmbeddingModelId,
        event: ModelProvisionProgress,
    },
    Finished {
        model: EmbeddingModelId,
        result: Result<(), FastSearchError>,
    },
}

enum PipelineMessage {
    Download(DownloadMessage),
    Indexing {
        model: EmbeddingModelId,
        stage: ComparisonModelStage,
    },
    Indexed {
        model: EmbeddingModelId,
        status: LifecycleStatus,
        error: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub struct ComparisonReadiness {
    model: EmbeddingModelId,
    weights_ready: bool,
    index_status: LifecycleStatus,
    index_metrics: Option<ModelPartitionMetrics>,
}

impl ComparisonReadiness {
    #[must_use]
    pub const fn model(&self) -> EmbeddingModelId {
        self.model
    }
    #[must_use]
    pub const fn weights_ready(&self) -> bool {
        self.weights_ready
    }
    #[must_use]
    pub const fn index_status(&self) -> &LifecycleStatus {
        &self.index_status
    }
    #[must_use]
    pub const fn index_metrics(&self) -> Option<ModelPartitionMetrics> {
        self.index_metrics
    }
    #[must_use]
    pub fn ready(&self) -> bool {
        self.weights_ready && self.index_status.freshness() == IndexFreshness::Current
    }
}

#[derive(Clone, Debug)]
pub struct ComparisonUpdateOutcome {
    model: EmbeddingModelId,
    status: LifecycleStatus,
    error: Option<String>,
}

impl ComparisonUpdateOutcome {
    #[must_use]
    pub const fn model(&self) -> EmbeddingModelId {
        self.model
    }
    #[must_use]
    pub const fn status(&self) -> &LifecycleStatus {
        &self.status
    }
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[derive(Clone, Debug)]
pub struct ComparisonModelResult {
    model: EmbeddingModelId,
    latency_ms: u128,
    hits: Vec<SearchHit>,
    error: Option<String>,
}

impl ComparisonModelResult {
    #[must_use]
    pub const fn model(&self) -> EmbeddingModelId {
        self.model
    }
    #[must_use]
    pub const fn latency_ms(&self) -> u128 {
        self.latency_ms
    }
    #[must_use]
    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[derive(Clone, Debug)]
pub struct ComparisonRun {
    lexical_hits: Vec<SearchHit>,
    models: Vec<ComparisonModelResult>,
}

impl ComparisonRun {
    #[must_use]
    pub fn lexical_hits(&self) -> &[SearchHit] {
        &self.lexical_hits
    }
    #[must_use]
    pub fn models(&self) -> &[ComparisonModelResult] {
        &self.models
    }
}

pub struct ComparisonCoordinator<'a> {
    runtime: &'a mut ProductionRuntime,
}

impl<'a> ComparisonCoordinator<'a> {
    #[must_use]
    pub const fn new(runtime: &'a mut ProductionRuntime) -> Self {
        Self { runtime }
    }

    pub fn readiness(&self) -> Result<Vec<ComparisonReadiness>, FastSearchError> {
        EmbeddingModelId::COMPARISON_ORDER
            .into_iter()
            .map(|model| {
                let cache = embedding_model_cache_status(model)?;
                Ok(ComparisonReadiness {
                    model,
                    weights_ready: cache.ready(),
                    index_status: self.runtime.model_partition_status(model),
                    index_metrics: self.runtime.model_partition_metrics(model).ok().flatten(),
                })
            })
            .collect()
    }

    /// One explicit action: reconcile shared sources once, then
    /// materialize only the model partitions that are not current.
    pub fn update_required(
        &mut self,
        show_download_progress: bool,
    ) -> Result<Vec<ComparisonUpdateOutcome>, FastSearchError> {
        self.update_required_with_progress(show_download_progress, |_| {})
    }

    pub(super) fn update_required_with_progress(
        &mut self,
        show_download_progress: bool,
        mut progress: impl FnMut(ComparisonUpdateProgress),
    ) -> Result<Vec<ComparisonUpdateOutcome>, FastSearchError> {
        let shared = self
            .runtime
            .index_shared_for_comparison_with_progress(|event| {
                progress(shared_progress(event));
            });
        if let Err(error) = shared {
            progress(ComparisonUpdateProgress::SharedFailed {
                message: error.message().to_owned(),
            });
            return Err(error);
        }
        progress(ComparisonUpdateProgress::SharedCompleted);
        let mut outcomes = Vec::with_capacity(EmbeddingModelId::COMPARISON_ORDER.len());
        let mut pending = Vec::new();
        for model in EmbeddingModelId::COMPARISON_ORDER {
            progress(ComparisonUpdateProgress::Model {
                model,
                stage: ComparisonModelStage::Checking,
            });
            let initial = match embedding_model_cache_status(model) {
                Ok(initial) => initial,
                Err(error) => {
                    progress(ComparisonUpdateProgress::Model {
                        model,
                        stage: ComparisonModelStage::Failed {
                            message: error.message().to_owned(),
                        },
                    });
                    return Err(error);
                }
            };
            let partition = self.runtime.model_partition_status(model);
            if initial.ready() && partition.freshness() == IndexFreshness::Current {
                progress(ComparisonUpdateProgress::Model {
                    model,
                    stage: ComparisonModelStage::Completed { reused: true },
                });
                outcomes.push(ComparisonUpdateOutcome {
                    model,
                    status: partition,
                    error: None,
                });
                continue;
            }
            progress(ComparisonUpdateProgress::Model {
                model,
                stage: ComparisonModelStage::Downloading {
                    asset: None,
                    completed_bytes: None,
                    total_bytes: None,
                },
            });
            pending.push(model);
        }

        provision_and_index_pipeline(
            self.runtime,
            &pending,
            show_download_progress,
            &mut progress,
            &mut outcomes,
        );
        outcomes.sort_by_key(|outcome| outcome.model.comparison_priority());
        Ok(outcomes)
    }

    pub fn run(&self, query: &SearchQuery, top_k: usize) -> Result<ComparisonRun, FastSearchError> {
        let shared = self.runtime.index_status();
        if shared.freshness() != IndexFreshness::Current {
            return Err(FastSearchError::new(
                crate::domain::ErrorKind::ProjectionFailure,
                "shared index is not current; update comparison indexes first",
            ));
        }
        let lexical_hits = self
            .runtime
            .lexical_baseline(query)?
            .hits()
            .iter()
            .take(top_k)
            .cloned()
            .collect();
        let mut models = Vec::with_capacity(EmbeddingModelId::COMPARISON_ORDER.len());
        let mut ready = Vec::with_capacity(EmbeddingModelId::COMPARISON_ORDER.len());
        for model in EmbeddingModelId::COMPARISON_ORDER {
            let started = Instant::now();
            let cache = embedding_model_cache_status(model)?;
            if !cache.ready() {
                models.push(ComparisonModelResult {
                    model,
                    latency_ms: started.elapsed().as_millis(),
                    hits: Vec::new(),
                    error: Some("model weights are absent".to_owned()),
                });
                continue;
            }
            ready.push((model, cache.root().to_path_buf()));
        }
        for (model, latency_ms, response) in self
            .runtime
            .search_model_partitions_parallel(&ready, query)?
        {
            match response {
                Ok(response) if response.freshness() == IndexFreshness::Current => {
                    models.push(ComparisonModelResult {
                        model,
                        latency_ms,
                        hits: response.hits().iter().take(top_k).cloned().collect(),
                        error: None,
                    });
                }
                Ok(response) => models.push(ComparisonModelResult {
                    model,
                    latency_ms,
                    hits: Vec::new(),
                    error: Some(format!(
                        "model partition is not current ({:?})",
                        response.freshness()
                    )),
                }),
                Err(error) => models.push(ComparisonModelResult {
                    model,
                    latency_ms,
                    hits: Vec::new(),
                    error: Some(error.message().to_owned()),
                }),
            }
        }
        models.sort_by_key(|result| result.model.comparison_priority());
        Ok(ComparisonRun {
            lexical_hits,
            models,
        })
    }
}

/// Keeps network transfers parallel while consuming successful downloads in
/// weakest-first order whenever more than one is ready. Exactly one validation/
/// indexing lane avoids RAM/VRAM contention.
fn provision_and_index_pipeline(
    runtime: &mut ProductionRuntime,
    models: &[EmbeddingModelId],
    show_download_progress: bool,
    progress: &mut impl FnMut(ComparisonUpdateProgress),
    outcomes: &mut Vec<ComparisonUpdateOutcome>,
) {
    if models.is_empty() {
        return;
    }
    let (sender, receiver) = mpsc::channel();
    let (index_sender, index_receiver) = mpsc::channel();
    thread::scope(|scope| {
        let index_events = sender.clone();
        scope.spawn(move || {
            for model in index_receiver {
                let _ = index_events.send(PipelineMessage::Indexing {
                    model,
                    stage: ComparisonModelStage::Validating,
                });
                let result =
                    ensure_embedding_model_with_progress(model, show_download_progress, |event| {
                        let _ = index_events.send(PipelineMessage::Download(
                            DownloadMessage::Progress { model, event },
                        ));
                    })
                    .and_then(|availability| {
                        let _ = index_events.send(PipelineMessage::Indexing {
                            model,
                            stage: ComparisonModelStage::PreparingForIndexing,
                        });
                        runtime.build_model_partition_with_progress(
                            model,
                            availability.root(),
                            |event| {
                                let stage = match event {
                                    VectorBuildProgress::Embedding {
                                        completed_records,
                                        total_records,
                                    } => ComparisonModelStage::Indexing {
                                        completed_records,
                                        total_records,
                                    },
                                    VectorBuildProgress::Saving => ComparisonModelStage::Saving,
                                };
                                let _ =
                                    index_events.send(PipelineMessage::Indexing { model, stage });
                            },
                        )
                    });
                let (status, error) = match result {
                    Ok(status) => (status, None),
                    Err(error) => (
                        runtime.model_partition_status(model),
                        Some(error.message().to_owned()),
                    ),
                };
                let _ = index_events.send(PipelineMessage::Indexed {
                    model,
                    status,
                    error,
                });
            }
        });
        for model in models.iter().copied() {
            let sender = sender.clone();
            scope.spawn(move || {
                let result = download_embedding_model_assets_with_progress(model, |event| {
                    let _ = sender.send(PipelineMessage::Download(DownloadMessage::Progress {
                        model,
                        event,
                    }));
                });
                let _ = sender.send(PipelineMessage::Download(DownloadMessage::Finished {
                    model,
                    result,
                }));
            });
        }
        drop(sender);
        let mut index_sender = Some(index_sender);
        let mut completed = 0;
        let mut downloads_finished = 0;
        let mut ready_for_indexing = Vec::new();
        let mut indexing = false;
        while completed < models.len() {
            let message = receiver
                .recv()
                .expect("comparison workers must report every requested model");
            match message {
                PipelineMessage::Download(DownloadMessage::Progress { model, event }) => {
                    progress(ComparisonUpdateProgress::Model {
                        model,
                        stage: ComparisonModelStage::Downloading {
                            asset: Some(event.asset().to_owned()),
                            completed_bytes: Some(event.completed_bytes()),
                            total_bytes: Some(event.total_bytes()),
                        },
                    });
                }
                PipelineMessage::Download(DownloadMessage::Finished { model, result }) => {
                    downloads_finished += 1;
                    if let Err(error) = result {
                        progress(ComparisonUpdateProgress::Model {
                            model,
                            stage: ComparisonModelStage::Failed {
                                message: error.message().to_owned(),
                            },
                        });
                        outcomes.push(ComparisonUpdateOutcome {
                            model,
                            status: LifecycleStatus::not_configured(error.message()),
                            error: Some(error.message().to_owned()),
                        });
                        completed += 1;
                    } else {
                        progress(ComparisonUpdateProgress::Model {
                            model,
                            stage: ComparisonModelStage::QueuedForIndexing,
                        });
                        ready_for_indexing.push(model);
                    }
                }
                PipelineMessage::Indexing { model, stage } => {
                    progress(ComparisonUpdateProgress::Model { model, stage });
                }
                PipelineMessage::Indexed {
                    model,
                    status,
                    error,
                } => {
                    let stage = match &error {
                        Some(message) => ComparisonModelStage::Failed {
                            message: message.clone(),
                        },
                        None => ComparisonModelStage::Completed { reused: false },
                    };
                    progress(ComparisonUpdateProgress::Model { model, stage });
                    outcomes.push(ComparisonUpdateOutcome {
                        model,
                        status,
                        error,
                    });
                    completed += 1;
                    indexing = false;
                }
            }
            if !indexing && !ready_for_indexing.is_empty() {
                let model = pop_next_index_model(&mut ready_for_indexing)
                    .expect("non-empty indexing queue has a next model");
                index_sender
                    .as_ref()
                    .expect("index lane stays open while a model is ready")
                    .send(model)
                    .expect("comparison index worker remains available");
                indexing = true;
            }
            if downloads_finished == models.len() && ready_for_indexing.is_empty() && !indexing {
                index_sender.take();
            }
        }
    });
}

fn pop_next_index_model(models: &mut Vec<EmbeddingModelId>) -> Option<EmbeddingModelId> {
    let next = models
        .iter()
        .enumerate()
        .min_by_key(|(_, model)| model.comparison_priority())
        .map(|(index, _)| index)?;
    Some(models.swap_remove(next))
}

#[cfg(test)]
fn run_model_downloads_in_parallel(
    models: &[EmbeddingModelId],
    progress: &mut impl FnMut(ComparisonUpdateProgress),
    download: impl Fn(
        EmbeddingModelId,
        &mut dyn FnMut(ModelProvisionProgress),
    ) -> Result<(), FastSearchError>
    + Sync,
) -> Vec<(EmbeddingModelId, Option<Result<(), FastSearchError>>)> {
    let (sender, receiver) = mpsc::channel();
    let mut results = models
        .iter()
        .copied()
        .map(|model| (model, None))
        .collect::<Vec<_>>();

    thread::scope(|scope| {
        for model in models.iter().copied() {
            let sender = sender.clone();
            let download = &download;
            scope.spawn(move || {
                let result = download(model, &mut |event| {
                    let _ = sender.send(DownloadMessage::Progress { model, event });
                });
                let _ = sender.send(DownloadMessage::Finished { model, result });
            });
        }
        drop(sender);
        for message in receiver {
            match message {
                DownloadMessage::Progress { model, event } => {
                    progress(ComparisonUpdateProgress::Model {
                        model,
                        stage: ComparisonModelStage::Downloading {
                            asset: Some(event.asset().to_owned()),
                            completed_bytes: Some(event.completed_bytes()),
                            total_bytes: Some(event.total_bytes()),
                        },
                    });
                }
                DownloadMessage::Finished { model, result } => {
                    let slot = results
                        .iter_mut()
                        .find(|(candidate, _)| *candidate == model)
                        .expect("parallel download result belongs to the requested model");
                    slot.1 = Some(result);
                }
            }
        }
    });
    results
}

fn shared_progress(progress: IndexingProgress) -> ComparisonUpdateProgress {
    ComparisonUpdateProgress::Shared {
        completed: progress.completed,
        total: progress.total,
        stage: match progress.stage {
            IndexingStage::Sources => ComparisonSharedStage::Sources,
            IndexingStage::State => ComparisonSharedStage::State,
            IndexingStage::Lexical => ComparisonSharedStage::Lexical,
            IndexingStage::Vector => unreachable!("shared comparison indexing excludes vectors"),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use super::*;

    #[test]
    fn model_download_scheduler_runs_network_jobs_concurrently() {
        let active = Arc::new(AtomicUsize::new(0));
        let maximum = Arc::new(AtomicUsize::new(0));
        let models = &EmbeddingModelId::ALL[2..5];
        let results = run_model_downloads_in_parallel(models, &mut |_| {}, {
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            move |_, _| {
                let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(current, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(40));
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            }
        });

        assert_eq!(results.len(), models.len());
        assert!(
            results
                .iter()
                .all(|(_, result)| result.as_ref().is_some_and(Result::is_ok))
        );
        assert!(maximum.load(Ordering::SeqCst) >= 2);
    }

    #[test]
    fn comparison_priority_selects_the_weakest_ready_model() {
        let mut ready = vec![
            EmbeddingModelId::BgeM3,
            EmbeddingModelId::MultilingualE5Large,
            EmbeddingModelId::MultilingualE5Base,
        ];

        assert_eq!(
            pop_next_index_model(&mut ready),
            Some(EmbeddingModelId::MultilingualE5Base)
        );
        assert_eq!(
            pop_next_index_model(&mut ready),
            Some(EmbeddingModelId::MultilingualE5Large)
        );
        assert_eq!(
            pop_next_index_model(&mut ready),
            Some(EmbeddingModelId::BgeM3)
        );
        assert_eq!(pop_next_index_model(&mut ready), None);
    }
}
