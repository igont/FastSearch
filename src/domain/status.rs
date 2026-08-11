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
    Unavailable { reason: String },
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
        }
    }
}
