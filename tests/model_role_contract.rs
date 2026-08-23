use std::{fs, path::Path, process::Command};

use fastsearch::{
    application::{
        ModelRuntimeIdentity, ModelSetReadySnapshot, PRODUCTION_MODEL_CATALOG, RoleReadinessMarker,
        model_set_identity_sha256, production_model_descriptor,
    },
    domain::ProductionModelRole,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn sha256(path: &Path) -> String {
    format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
}

fn marker(role: ProductionModelRole) -> RoleReadinessMarker {
    let descriptor = production_model_descriptor(role);
    let runtimes = ProductionModelRole::ALL
        .into_iter()
        .map(|role| {
            (
                role,
                ModelRuntimeIdentity::qualified(role, "11".repeat(32)).unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let set_identity = model_set_identity_sha256(&runtimes).unwrap();
    RoleReadinessMarker::new(
        role,
        descriptor.repository,
        descriptor.revision,
        descriptor.compute_contract_sha256(),
        descriptor.manifest_sha256(),
        set_identity,
        ModelRuntimeIdentity::qualified(role, "11".repeat(32)).unwrap(),
    )
    .unwrap()
}

#[test]
fn production_catalog_matches_the_accepted_four_role_manifest() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest: Value = serde_json::from_slice(
        &fs::read(root.join("evidence/dt4/fixtures/ts-dt4-01/model-manifest.json")).unwrap(),
    )
    .unwrap();
    let expected = manifest["models"].as_array().unwrap();
    assert_eq!(expected.len(), PRODUCTION_MODEL_CATALOG.len());
    for (actual, expected) in PRODUCTION_MODEL_CATALOG.iter().zip(expected) {
        assert_eq!(actual.role.slug(), expected["slug"].as_str().unwrap());
        assert_eq!(actual.repository, expected["repository"].as_str().unwrap());
        assert_eq!(actual.revision, expected["revision"].as_str().unwrap());
        assert_eq!(
            actual.required_files.len(),
            expected["assets"].as_array().unwrap().len()
        );
        for (actual_file, expected_file) in actual
            .required_files
            .iter()
            .zip(expected["assets"].as_array().unwrap())
        {
            assert_eq!(actual_file.path, expected_file["path"].as_str().unwrap());
            assert_eq!(actual_file.bytes, expected_file["bytes"].as_u64().unwrap());
            assert_eq!(
                actual_file.sha256,
                expected_file["sha256"].as_str().unwrap()
            );
        }
    }
}

#[test]
fn snapshot_round_trip_preserves_one_deterministic_generation() {
    let snapshot =
        ModelSetReadySnapshot::new(ProductionModelRole::ALL.into_iter().map(marker).collect())
            .unwrap();
    let restored = ModelSetReadySnapshot::from_json(&snapshot.to_json()).unwrap();
    assert_eq!(restored, snapshot);
    assert_eq!(snapshot.roles().len(), 4);
    assert_eq!(snapshot.generation().len(), 64);
}

#[test]
fn independent_oracle_artifact_is_self_identifying_and_covers_four_probes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let oracle_path = root.join("evidence/dt4/fixtures/ts-dt4-02/oracle.json");
    let oracle: Value = serde_json::from_slice(&fs::read(&oracle_path).unwrap()).unwrap();
    assert_eq!(oracle["schema"], 1);
    assert_eq!(
        oracle["script_sha256"],
        sha256(&root.join("scripts/dt4_model_readiness_oracle.py"))
    );
    assert_eq!(
        oracle["requirements_sha256"],
        sha256(&root.join("evidence/dt4/oracle/requirements.lock"))
    );
    assert_eq!(
        oracle["cases_sha256"],
        sha256(&root.join("evidence/dt4/fixtures/ts-dt4-02/oracle-cases.json"))
    );
    assert_eq!(
        oracle["model_manifest_sha256"],
        sha256(&root.join("evidence/dt4/fixtures/ts-dt4-01/model-manifest.json"))
    );
    let observations = oracle["observations"].as_object().unwrap();
    assert_eq!(observations.len(), 4);
    assert_eq!(observations["arctic-embed-l-v2"]["dimension"], 1024);
    assert_eq!(observations["multilingual-e5-large"]["dimension"], 1024);
    assert_eq!(observations["nomic-embed-text-v2-moe"]["dimension"], 768);
    for role in [
        "arctic-embed-l-v2",
        "multilingual-e5-large",
        "nomic-embed-text-v2-moe",
    ] {
        let norm = observations[role]["query_norm"].as_f64().unwrap();
        assert!((norm - 1.0).abs() <= 0.00001, "{role}: {norm}");
        assert_eq!(
            observations[role]["query_components"]
                .as_array()
                .unwrap()
                .len(),
            8
        );
        assert_eq!(observations[role]["order"].as_array().unwrap().len(), 2);
    }
    assert_eq!(
        observations["qwen3-reranker-0.6b"]["order"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn public_model_status_rejects_unprepared_state_without_a_workspace() {
    let binary = env!("CARGO_BIN_EXE_fastsearch");
    let home = std::env::temp_dir().join(format!("fastsearch-model-status-{}", std::process::id()));
    if home.exists() {
        fs::remove_dir_all(&home).unwrap();
    }
    let model_root = home.join("models").join("production");
    fs::create_dir_all(&model_root).unwrap();

    let output = Command::new(binary)
        .args(["models", "status", "--json"])
        .env("FASTSEARCH_HOME", &home)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let report: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(report["status"], "error");
    assert_eq!(report["error"]["code"], "model_readiness_failed");
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .starts_with("MODEL_SET_NOT_READY:")
    );
    assert!(!home.join("catalog.json").exists());
    assert!(!home.join("workspaces").exists());

    fs::remove_dir_all(home).unwrap();
}
