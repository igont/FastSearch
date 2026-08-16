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
}

impl EmbeddingModelId {
    pub const ALL: [Self; 5] = [
        Self::MultilingualE5Small,
        Self::MultilingualE5Base,
        Self::MultilingualE5Large,
        Self::Qwen3Embedding06B,
        Self::NomicEmbedTextV2Moe,
    ];

    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::MultilingualE5Small => "multilingual-e5-small",
            Self::MultilingualE5Base => "multilingual-e5-base",
            Self::MultilingualE5Large => "multilingual-e5-large",
            Self::Qwen3Embedding06B => "qwen3-embedding-0.6b",
            Self::NomicEmbedTextV2Moe => "nomic-embed-text-v2-moe",
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
        }
    }

    #[must_use]
    pub const fn dimension(self) -> usize {
        match self {
            Self::MultilingualE5Small => 384,
            Self::MultilingualE5Base | Self::NomicEmbedTextV2Moe => 768,
            Self::MultilingualE5Large | Self::Qwen3Embedding06B => 1024,
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
}
