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
    Validating,
    WaitingForIndex,
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
        EmbeddingModelId::ALL
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
        let mut outcomes = Vec::with_capacity(EmbeddingModelId::ALL.len());
        let mut pending = Vec::new();
        for model in EmbeddingModelId::ALL {
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

        let mut downloads = download_model_assets_in_parallel(&pending, &mut progress);
        for (model, result) in &downloads {
            if result.as_ref().is_some_and(Result::is_ok) {
                progress(ComparisonUpdateProgress::Model {
                    model: *model,
                    stage: ComparisonModelStage::WaitingForIndex,
                });
            }
        }
        for model in pending {
            let download = downloads
                .iter_mut()
                .find(|(candidate, _)| *candidate == model)
                .and_then(|(_, result)| result.take())
                .expect("parallel downloader returns one result per requested model");
            let result = download.and_then(|()| {
                progress(ComparisonUpdateProgress::Model {
                    model,
                    stage: ComparisonModelStage::Validating,
                });
                ensure_embedding_model_with_progress(model, show_download_progress, |_| {})
            });
            let result = result.and_then(|availability| {
                progress(ComparisonUpdateProgress::Model {
                    model,
                    stage: ComparisonModelStage::Indexing {
                        completed_records: 0,
                        total_records: 0,
                    },
                });
                self.runtime.build_model_partition_with_progress(
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
                        progress(ComparisonUpdateProgress::Model { model, stage });
                    },
                )
            });
            match result {
                Ok(status) => {
                    progress(ComparisonUpdateProgress::Model {
                        model,
                        stage: ComparisonModelStage::Completed { reused: false },
                    });
                    outcomes.push(ComparisonUpdateOutcome {
                        model,
                        status,
                        error: None,
                    });
                }
                Err(error) => {
                    progress(ComparisonUpdateProgress::Model {
                        model,
                        stage: ComparisonModelStage::Failed {
                            message: error.message().to_owned(),
                        },
                    });
                    outcomes.push(ComparisonUpdateOutcome {
                        model,
                        status: self.runtime.model_partition_status(model),
                        error: Some(error.message().to_owned()),
                    });
                }
            }
        }
        outcomes.sort_by_key(|outcome| {
            EmbeddingModelId::ALL
                .iter()
                .position(|model| *model == outcome.model)
                .unwrap_or(EmbeddingModelId::ALL.len())
        });
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
        let mut models = Vec::with_capacity(EmbeddingModelId::ALL.len());
        for model in EmbeddingModelId::ALL {
            let started = Instant::now();
            let cache = embedding_model_cache_status(model)?;
            let partition = self.runtime.model_partition_status(model);
            if !cache.ready() || partition.freshness() != IndexFreshness::Current {
                let reason = if !cache.ready() {
                    "model weights are absent"
                } else {
                    partition.detail()
                };
                models.push(ComparisonModelResult {
                    model,
                    latency_ms: started.elapsed().as_millis(),
                    hits: Vec::new(),
                    error: Some(reason.to_owned()),
                });
                continue;
            }
            match self
                .runtime
                .search_model_partition(model, cache.root(), query)
            {
                Ok(response) => models.push(ComparisonModelResult {
                    model,
                    latency_ms: started.elapsed().as_millis(),
                    hits: response.hits().iter().take(top_k).cloned().collect(),
                    error: None,
                }),
                Err(error) => models.push(ComparisonModelResult {
                    model,
                    latency_ms: started.elapsed().as_millis(),
                    hits: Vec::new(),
                    error: Some(error.message().to_owned()),
                }),
            }
        }
        Ok(ComparisonRun {
            lexical_hits,
            models,
        })
    }
}

fn download_model_assets_in_parallel(
    models: &[EmbeddingModelId],
    progress: &mut impl FnMut(ComparisonUpdateProgress),
) -> Vec<(EmbeddingModelId, Option<Result<(), FastSearchError>>)> {
    run_model_downloads_in_parallel(models, progress, |model, report| {
        download_embedding_model_assets_with_progress(model, report)
    })
}

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
}
