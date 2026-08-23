use serde::{Deserialize, Serialize};

use super::EmbeddingModelId;

/// One required role in the production model set.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProductionModelRole {
    ArcticEmbedLV2,
    MultilingualE5Large,
    NomicEmbedTextV2Moe,
    Qwen3Reranker06B,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelRoleKind {
    Embedding,
    Reranker,
}

impl ProductionModelRole {
    /// Canonical order used by the catalog and model-set generation hash.
    pub const ALL: [Self; 4] = [
        Self::ArcticEmbedLV2,
        Self::MultilingualE5Large,
        Self::NomicEmbedTextV2Moe,
        Self::Qwen3Reranker06B,
    ];

    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::ArcticEmbedLV2 => "arctic-embed-l-v2",
            Self::MultilingualE5Large => "multilingual-e5-large",
            Self::NomicEmbedTextV2Moe => "nomic-embed-text-v2-moe",
            Self::Qwen3Reranker06B => "qwen3-reranker-0.6b",
        }
    }

    #[must_use]
    pub const fn kind(self) -> ModelRoleKind {
        match self {
            Self::ArcticEmbedLV2 | Self::MultilingualE5Large | Self::NomicEmbedTextV2Moe => {
                ModelRoleKind::Embedding
            }
            Self::Qwen3Reranker06B => ModelRoleKind::Reranker,
        }
    }

    #[must_use]
    pub const fn embedding_model(self) -> Option<EmbeddingModelId> {
        match self {
            Self::ArcticEmbedLV2 => Some(EmbeddingModelId::SnowflakeArcticEmbedLV2),
            Self::MultilingualE5Large => Some(EmbeddingModelId::MultilingualE5Large),
            Self::NomicEmbedTextV2Moe => Some(EmbeddingModelId::NomicEmbedTextV2Moe),
            Self::Qwen3Reranker06B => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_roles_do_not_confuse_qwen_embedding_and_reranker() {
        assert_eq!(ProductionModelRole::ALL.len(), 4);
        assert_eq!(
            ProductionModelRole::Qwen3Reranker06B.kind(),
            ModelRoleKind::Reranker
        );
        assert_eq!(
            ProductionModelRole::Qwen3Reranker06B.embedding_model(),
            None
        );
        assert!(
            !ProductionModelRole::ALL.iter().any(|role| {
                role.embedding_model() == Some(EmbeddingModelId::Qwen3Embedding06B)
            })
        );
    }
}
