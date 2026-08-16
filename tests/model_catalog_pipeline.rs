use fastsearch::{
    application::{MODEL_CATALOG, ensure_embedding_model},
    domain::EmbeddingModelId,
};

#[test]
#[ignore = "downloads and opens every catalog model; requires network, disk and several GB RAM"]
fn every_catalog_model_downloads_and_passes_readiness_without_indexing() {
    for descriptor in MODEL_CATALOG {
        let availability = ensure_embedding_model(descriptor.id, false)
            .unwrap_or_else(|error| panic!("{}: {}", descriptor.id.slug(), error.message()));
        assert_eq!(availability.model(), descriptor.id);
        assert!(availability.root().is_dir());
    }
    assert_eq!(MODEL_CATALOG.len(), EmbeddingModelId::ALL.len());
}
