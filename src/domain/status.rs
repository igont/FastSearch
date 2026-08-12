use super::{ErrorKind, FastSearchError};

/// Объявленная capability общего core.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    Source,
    State,
    LexicalRetrieval,
    VectorRetrieval,
    CodeMaps,
    Symbols,
    AgentSurface,
}

/// Тип реально подключённого backend; unavailable не маскируется как backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackendKind {
    Mock,
    Real,
}

/// Наблюдаемое состояние capability.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CapabilityState {
    Available { backend: BackendKind },
    Stale { detail: String },
    Degraded { detail: String },
    Unavailable { reason: String },
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IndexFreshness {
    #[default]
    NotConfigured,
    Current,
    Stale,
    Degraded,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LifecycleStatus {
    freshness: IndexFreshness,
    state_generation: u64,
    projection_generation: Option<u64>,
    detail: String,
}

impl LifecycleStatus {
    #[must_use]
    pub fn not_configured(detail: impl Into<String>) -> Self {
        Self {
            freshness: IndexFreshness::NotConfigured,
            state_generation: 0,
            projection_generation: None,
            detail: detail.into(),
        }
    }
    #[must_use]
    pub fn new(
        freshness: IndexFreshness,
        state_generation: u64,
        projection_generation: Option<u64>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            freshness,
            state_generation,
            projection_generation,
            detail: detail.into(),
        }
    }
    #[must_use]
    pub const fn freshness(&self) -> IndexFreshness {
        self.freshness
    }
    #[must_use]
    pub const fn state_generation(&self) -> u64 {
        self.state_generation
    }
    #[must_use]
    pub const fn projection_generation(&self) -> Option<u64> {
        self.projection_generation
    }
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

/// Статус одной capability для CLI, будущего agent surface и tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityStatus {
    capability: Capability,
    state: CapabilityState,
}

impl CapabilityStatus {
    #[must_use]
    pub const fn available(capability: Capability, backend: BackendKind) -> Self {
        Self {
            capability,
            state: CapabilityState::Available { backend },
        }
    }

    #[must_use]
    pub fn unavailable(capability: Capability, reason: impl Into<String>) -> Self {
        Self {
            capability,
            state: CapabilityState::Unavailable {
                reason: reason.into(),
            },
        }
    }

    #[must_use]
    pub fn stale(capability: Capability, detail: impl Into<String>) -> Self {
        Self {
            capability,
            state: CapabilityState::Stale {
                detail: detail.into(),
            },
        }
    }

    #[must_use]
    pub fn degraded(capability: Capability, detail: impl Into<String>) -> Self {
        Self {
            capability,
            state: CapabilityState::Degraded {
                detail: detail.into(),
            },
        }
    }

    #[must_use]
    pub const fn capability(&self) -> Capability {
        self.capability
    }
    #[must_use]
    pub const fn state(&self) -> &CapabilityState {
        &self.state
    }

    pub fn require_available(&self) -> Result<BackendKind, FastSearchError> {
        match &self.state {
            CapabilityState::Available { backend } => Ok(*backend),
            CapabilityState::Unavailable { reason } => Err(FastSearchError::new(
                ErrorKind::CapabilityUnavailable {
                    capability: self.capability,
                },
                reason.clone(),
            )),
            CapabilityState::Stale { detail } | CapabilityState::Degraded { detail } => {
                Err(FastSearchError::new(
                    ErrorKind::CapabilityUnavailable {
                        capability: self.capability,
                    },
                    detail.clone(),
                ))
            }
        }
    }
}
