use serde::{Deserialize, Serialize};

/// One mutually-exclusive dense embedding model used by a workspace.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EmbeddingModelId {
    #[default]
    MultilingualE5Small,
    MultilingualE5Base,
    MultilingualE5Large,
    #[serde(rename = "qwen3-embedding-0.6b")]
    Qwen3Embedding06B,
    NomicEmbedTextV2Moe,
    SnowflakeArcticEmbedLV2,
    GteMultilingualBase,
    BgeM3,
    JinaEmbeddingsV3,
}

impl EmbeddingModelId {
    pub const ALL: [Self; 9] = [
        Self::MultilingualE5Small,
        Self::MultilingualE5Base,
        Self::MultilingualE5Large,
        Self::Qwen3Embedding06B,
        Self::NomicEmbedTextV2Moe,
        Self::SnowflakeArcticEmbedLV2,
        Self::GteMultilingualBase,
        Self::BgeM3,
        Self::JinaEmbeddingsV3,
    ];

    /// Presentation and experiment order: declared retrieval capability first,
    /// then lower-cost baselines. The unavailable Jina runtime stays last.
    pub const DISPLAY_ORDER: [Self; 9] = [
        Self::BgeM3,
        Self::SnowflakeArcticEmbedLV2,
        Self::Qwen3Embedding06B,
        Self::MultilingualE5Large,
        Self::NomicEmbedTextV2Moe,
        Self::GteMultilingualBase,
        Self::MultilingualE5Base,
        Self::MultilingualE5Small,
        Self::JinaEmbeddingsV3,
    ];

    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::MultilingualE5Small => "multilingual-e5-small",
            Self::MultilingualE5Base => "multilingual-e5-base",
            Self::MultilingualE5Large => "multilingual-e5-large",
            Self::Qwen3Embedding06B => "qwen3-embedding-0.6b",
            Self::NomicEmbedTextV2Moe => "nomic-embed-text-v2-moe",
            Self::SnowflakeArcticEmbedLV2 => "arctic-embed-l-v2",
            Self::GteMultilingualBase => "gte-multilingual-base",
            Self::BgeM3 => "bge-m3",
            Self::JinaEmbeddingsV3 => "jina-embeddings-v3",
        }
    }

    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::MultilingualE5Small => "E5 Small",
            Self::MultilingualE5Base => "E5 Base",
            Self::MultilingualE5Large => "E5 Large",
            Self::Qwen3Embedding06B => "Qwen3 Embedding 0.6B",
            Self::NomicEmbedTextV2Moe => "Nomic Embed Text v2 MoE",
            Self::SnowflakeArcticEmbedLV2 => "Arctic Embed L v2",
            Self::GteMultilingualBase => "GTE Multilingual Base",
            Self::BgeM3 => "BGE-M3",
            Self::JinaEmbeddingsV3 => "Jina Embeddings v3",
        }
    }

    #[must_use]
    pub const fn dimension(self) -> usize {
        match self {
            Self::MultilingualE5Small => 384,
            Self::MultilingualE5Base | Self::NomicEmbedTextV2Moe | Self::GteMultilingualBase => 768,
            Self::MultilingualE5Large
            | Self::Qwen3Embedding06B
            | Self::SnowflakeArcticEmbedLV2
            | Self::BgeM3
            | Self::JinaEmbeddingsV3 => 1024,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let value = value.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|model| {
            value == model.slug()
                || value == model.display_name().to_ascii_lowercase()
                || match model {
                    Self::MultilingualE5Small => matches!(value.as_str(), "small" | "e5-small"),
                    Self::MultilingualE5Base => matches!(value.as_str(), "base" | "e5-base"),
                    Self::MultilingualE5Large => matches!(value.as_str(), "large" | "e5-large"),
                    Self::Qwen3Embedding06B => matches!(value.as_str(), "qwen" | "qwen3"),
                    Self::NomicEmbedTextV2Moe => matches!(value.as_str(), "nomic" | "nomic-v2"),
                    Self::SnowflakeArcticEmbedLV2 => {
                        matches!(value.as_str(), "arctic" | "snowflake")
                    }
                    Self::GteMultilingualBase => matches!(value.as_str(), "gte" | "gte-base"),
                    Self::BgeM3 => matches!(value.as_str(), "bge" | "m3"),
                    Self::JinaEmbeddingsV3 => matches!(value.as_str(), "jina" | "jina-v3"),
                }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_slugs_round_trip_through_parser() {
        for model in EmbeddingModelId::ALL {
            assert_eq!(EmbeddingModelId::parse(model.slug()), Some(model));
        }
    }

    #[test]
    fn display_order_contains_every_stable_model_once() {
        let mut models = EmbeddingModelId::DISPLAY_ORDER.to_vec();
        models.sort_by_key(|model| model.slug());
        let mut all = EmbeddingModelId::ALL.to_vec();
        all.sort_by_key(|model| model.slug());
        assert_eq!(models, all);
    }
}
