#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
        sync::{Mutex, atomic::AtomicBool},
        time::Instant,
    };

    use serde_json::Value;
    use sha2::{Digest, Sha256};

    use crate::{
        application::{
            ProductionRuntime, PublicSearchRequest, ThinSearchCoordinator, WorkspaceStore,
            production_model_set_status,
        },
        domain::EmbeddingModelId,
    };

    static ENVIRONMENT: Mutex<()> = Mutex::new(());
    const MODELS: [EmbeddingModelId; 3] = [
        EmbeddingModelId::SnowflakeArcticEmbedLV2,
        EmbeddingModelId::MultilingualE5Large,
        EmbeddingModelId::NomicEmbedTextV2Moe,
    ];
    const QWEN_TOLERANCE: f64 = 0.000_01;

    #[test]
    #[ignore = "requires pinned TS-DT4-01 model-cache template"]
    fn three_projection_candidates_match_independent_oracle() {
        let _environment = ENVIRONMENT.lock().expect("environment lock");
        let fixture = required_path("FASTSEARCH_DT4_FIXTURE_ROOT");
        let product_home = required_path("FASTSEARCH_DT4_MODEL_CACHE");
        let oracle_path = required_path("FASTSEARCH_DT4_ORACLE");
        verify_model_cache(&fixture, &product_home);
        let temp = unique_temp("intermediate");
        let workspace = temp.join("workspace");
        copy_tree(&fixture.join("prepared-workspace"), &workspace);
        let _environment_guard = EnvironmentGuard::install(&product_home);

        {
            let store = WorkspaceStore::open(&workspace).expect("prepared workspace opens");
            let runtime = ProductionRuntime::open(store.production_config())
                .expect("production runtime opens");
            for model in MODELS {
                production_model_set_status().expect("production model set is ready");
                let status = runtime.model_partition_status(model);
                assert_eq!(status.state_generation(), 1);
                assert_eq!(status.projection_generation(), Some(1));
            }

            let query = fs::read_to_string(fixture.join("query.txt")).expect("query");
            let request = PublicSearchRequest::new(query, None, None).expect("public request");
            let mut coordinator = ThinSearchCoordinator::open(&workspace).expect("coordinator");
            let (response, audit) = coordinator
                .search(&request, &AtomicBool::new(false), Instant::now())
                .expect("real three-projection search");
            let oracle: Value = serde_json::from_slice(&fs::read(oracle_path).expect("oracle"))
                .expect("oracle JSON");

            assert_eq!(audit.candidate_slots, 15);
            let expected_candidates = oracle["embedding_candidates"]
                .as_object()
                .expect("embedding candidates");
            for (model, expected) in expected_candidates {
                let actual = audit
                    .candidates_by_model
                    .get(model)
                    .expect("model candidates");
                let expected = expected.as_array().expect("ordered candidates");
                assert_eq!(actual.len(), 5);
                assert_eq!(expected.len(), 5);
                for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
                    assert_eq!(actual, expected["stable_id"].as_str().expect("stable id"));
                    assert_eq!(index + 1, expected["rank"].as_u64().expect("rank") as usize);
                }
            }
            let expected_dedup = oracle["deduplicated_stable_ids"]
                .as_array()
                .expect("ordered dedup")
                .iter()
                .map(|value| value.as_str().expect("dedup id"))
                .collect::<Vec<_>>();
            assert_eq!(
                audit
                    .deduplicated_stable_ids
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
                expected_dedup
            );
            assert_eq!(audit.unique_candidates, expected_dedup.len());

            let expected_probabilities = oracle["qwen_probabilities"]
                .as_object()
                .expect("Qwen probabilities");
            assert_eq!(audit.qwen_probabilities.len(), expected_probabilities.len());
            for (id, expected) in expected_probabilities {
                let actual = f64::from(*audit.qwen_probabilities.get(id).expect("Qwen score"));
                let expected = expected.as_f64().expect("Python Qwen probability");
                assert!(
                    (actual - expected).abs() <= QWEN_TOLERANCE,
                    "Qwen probability for {id}: Rust {actual}, Python {expected}"
                );
            }

            let public = serde_json::to_value(response).expect("public response");
            let actual_results = public["results"].as_array().expect("public results");
            let expected_results = oracle["results"].as_array().expect("oracle results");
            let cutoff: Value = serde_json::from_str(include_str!(
                "../../tests/fixtures/relevance/evaluation.json"
            ))
            .unwrap();
            let expected_results = expected_results
                .iter()
                .filter(|row| {
                    oracle["qwen_probabilities"][row["stable_id"].as_str().unwrap()]
                        .as_f64()
                        .unwrap()
                        >= cutoff["threshold"].as_f64().unwrap()
                })
                .take(5)
                .collect::<Vec<_>>();
            assert!(actual_results.len() <= 5);
            assert_eq!(actual_results.len(), expected_results.len());
            for (actual, expected) in actual_results.iter().zip(expected_results) {
                assert_eq!(actual["rank"], expected["rank"]);
                assert_eq!(actual["title"], expected["title"]);
                assert!(
                    actual["path"]
                        .as_str()
                        .expect("absolute path")
                        .ends_with(expected["normalized_path"].as_str().expect("relative path"))
                );
                assert_eq!(
                    format!(
                        "{:x}",
                        Sha256::digest(actual["content"].as_str().expect("content").as_bytes())
                    ),
                    expected["content_sha256"].as_str().expect("content hash")
                );
            }
        }
        let _ = fs::remove_dir_all(temp);
    }

    struct EnvironmentGuard {
        home: Option<std::ffi::OsString>,
        hf: Option<std::ffi::OsString>,
        offline: Option<std::ffi::OsString>,
    }

    impl EnvironmentGuard {
        fn install(product_home: &Path) -> Self {
            let guard = Self {
                home: env::var_os("FASTSEARCH_HOME"),
                hf: env::var_os("HF_HOME"),
                offline: env::var_os("HF_HUB_OFFLINE"),
            };
            // SAFETY: the ignored test owns ENVIRONMENT until this guard is dropped.
            unsafe {
                env::set_var("FASTSEARCH_HOME", product_home);
                env::set_var("HF_HOME", product_home.join("models").join("huggingface"));
                env::set_var("HF_HUB_OFFLINE", "1");
            }
            guard
        }
    }

    impl Drop for EnvironmentGuard {
        fn drop(&mut self) {
            // SAFETY: the ignored test still owns ENVIRONMENT while the guard is dropped.
            unsafe {
                restore("FASTSEARCH_HOME", self.home.take());
                restore("HF_HOME", self.hf.take());
                restore("HF_HUB_OFFLINE", self.offline.take());
            }
        }
    }

    unsafe fn restore(name: &str, value: Option<std::ffi::OsString>) {
        if let Some(value) = value {
            unsafe { env::set_var(name, value) };
        } else {
            unsafe { env::remove_var(name) };
        }
    }

    fn required_path(name: &str) -> PathBuf {
        fs::canonicalize(env::var_os(name).unwrap_or_else(|| panic!("{name} is required")))
            .unwrap_or_else(|error| panic!("{name} must resolve: {error}"))
    }

    fn unique_temp(label: &str) -> PathBuf {
        let root = env::temp_dir().join(format!(
            "fastsearch-dt4-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("temporary root");
        root
    }

    fn copy_tree(source: &Path, target: &Path) {
        fs::create_dir_all(target).expect("target directory");
        for entry in fs::read_dir(source).expect("source directory") {
            let entry = entry.expect("source entry");
            let destination = target.join(entry.file_name());
            if entry.file_type().expect("entry type").is_dir() {
                copy_tree(&entry.path(), &destination);
            } else {
                fs::copy(entry.path(), destination).expect("fixture copy");
            }
        }
    }

    fn verify_model_cache(fixture: &Path, product_home: &Path) {
        let manifest: Value = serde_json::from_slice(
            &fs::read(fixture.join("model-manifest.json")).expect("model manifest"),
        )
        .expect("model manifest JSON");
        for model in manifest["models"].as_array().expect("models") {
            let slug = model["slug"].as_str().expect("model slug");
            let repository = model["repository"].as_str().expect("repository");
            let revision = model["revision"].as_str().expect("revision");
            let repository_dir = format!("models--{}", repository.replace('/', "--"));
            let snapshot = product_home
                .join("models/production/artifacts")
                .join(slug)
                .join("hub")
                .join(repository_dir)
                .join("snapshots")
                .join(revision);

            for asset in model["assets"].as_array().expect("model assets") {
                let path = snapshot.join(asset["path"].as_str().expect("asset path"));
                assert_eq!(
                    fs::metadata(&path).expect("pinned model asset").len(),
                    asset["bytes"].as_u64().expect("bytes")
                );
                assert_eq!(sha256(&path), asset["sha256"].as_str().expect("asset hash"));
            }
        }
    }

    fn sha256(path: &Path) -> String {
        use std::io::Read;

        let mut file = fs::File::open(path).expect("pinned model asset");
        let mut digest = Sha256::new();
        let mut buffer = vec![0_u8; 1024 * 1024];
        loop {
            let read = file.read(&mut buffer).expect("read model asset");
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        format!("{:x}", digest.finalize())
    }
}
