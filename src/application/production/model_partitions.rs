use super::*;

impl ProductionRuntime {
    /// Read-only readiness of one model partition against the current shared
    /// canonical state. This never opens or downloads model weights.
    pub fn model_partition_status(&self, model: EmbeddingModelId) -> LifecycleStatus {
        let records = match self
            .state
            .all_records()
            .and_then(|records| project_records(&records, model).map(|corpus| corpus.records))
        {
            Ok(records) => records,
            Err(error) => {
                return LifecycleStatus::new(IndexFreshness::Degraded, 0, None, error.message());
            }
        };
        let generation = match self.state.durable_generation() {
            Ok(generation) => generation,
            Err(error) => {
                return LifecycleStatus::new(IndexFreshness::Degraded, 0, None, error.message());
            }
        };
        LocalE5Vector::persistent_status(
            &model_partition_root(self.service.service_root(), model),
            model,
            &super::super::model_cache::model_identity(model),
            &records,
            generation,
        )
    }

    pub fn model_partition_metrics(
        &self,
        model: EmbeddingModelId,
    ) -> Result<Option<ModelPartitionMetrics>, FastSearchError> {
        Ok(LocalE5Vector::persistent_metrics(&model_partition_root(
            self.service.service_root(),
            model,
        ))?
        .map(|metrics| ModelPartitionMetrics {
            size_bytes: metrics.size_bytes(),
            build_duration_ms: metrics.build_duration_ms(),
        }))
    }

    pub fn build_model_partition(
        &self,
        model: EmbeddingModelId,
        model_root: &Path,
    ) -> Result<LifecycleStatus, FastSearchError> {
        self.build_model_partition_with_progress(model, model_root, |_| {})
    }

    pub(crate) fn build_model_partition_with_progress(
        &self,
        model: EmbeddingModelId,
        model_root: &Path,
        mut progress: impl FnMut(VectorBuildProgress),
    ) -> Result<LifecycleStatus, FastSearchError> {
        if !self.workspace_layout {
            return Err(FastSearchError::new(
                ErrorKind::InvalidContent,
                "model partitions require a workspace layout",
            ));
        }
        let records = project_records(&self.state.all_records()?, model)?.records;
        let generation = self.state.durable_generation()?;
        let vector = LocalE5Vector::open_persistent_with_model_on_device(
            model_root,
            super::super::model_cache::model_identity(model),
            model,
            model_partition_root(self.service.service_root(), model),
            super::super::model_cache::configured_model_device(model)?,
        );
        vector.restore(&records, generation)?;
        vector.apply_with_progress(&records, generation, &mut progress)
    }

    pub fn search_model_partition(
        &self,
        model: EmbeddingModelId,
        model_root: &Path,
        query: &SearchQuery,
    ) -> Result<SearchResponse, FastSearchError> {
        let records = project_records(&self.state.all_records()?, model)?.records;
        let generation = self.state.durable_generation()?;
        let vector = LocalE5Vector::open_persistent_with_model_on_device(
            model_root,
            super::super::model_cache::model_identity(model),
            model,
            model_partition_root(self.service.service_root(), model),
            super::super::model_cache::configured_model_device(model)?,
        );
        vector.restore(&records, generation)?;
        canonicalize_projection_hits(&self.state, vector.search(query)?)
    }

    pub(crate) fn search_model_partitions_parallel(
        &self,
        requests: &[(EmbeddingModelId, PathBuf)],
        query: &SearchQuery,
    ) -> Result<Vec<ModelPartitionSearchOutcome>, FastSearchError> {
        let canonical_records = self.state.all_records()?;
        let generation = self.state.durable_generation()?;
        let service_root = self.service.service_root().to_path_buf();
        let attempts = run_jobs_resilient(requests, |(model, model_root)| {
            super::super::model_cache::configured_model_device(*model).and_then(|device| {
                let records = project_records(&canonical_records, *model)?.records;
                let vector = LocalE5Vector::open_persistent_with_model_on_device(
                    model_root,
                    super::super::model_cache::model_identity(*model),
                    *model,
                    model_partition_root(&service_root, *model),
                    device,
                );
                vector.restore(&records, generation)?;
                vector.search(query)
            })
        });
        let mut outcomes = requests
            .iter()
            .zip(attempts)
            .map(|((model, _), (latency_ms, response))| {
                (
                    *model,
                    latency_ms,
                    response
                        .and_then(|response| canonicalize_projection_hits(&self.state, response)),
                )
            })
            .collect::<Vec<_>>();
        outcomes.sort_by_key(|(model, _, _)| {
            EmbeddingModelId::DISPLAY_ORDER
                .iter()
                .position(|candidate| candidate == model)
                .unwrap_or(EmbeddingModelId::DISPLAY_ORDER.len())
        });
        Ok(outcomes)
    }

    pub fn lexical_baseline(&self, query: &SearchQuery) -> Result<SearchResponse, FastSearchError> {
        canonicalize_projection_hits(&self.state, self.lexical.search(query)?)
    }
}
