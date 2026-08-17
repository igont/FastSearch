//! Runtime execution targets and measured local capabilities.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionDevice {
    #[default]
    Cpu,
    #[serde(rename = "gpu")]
    GpuDirectMl,
}

impl ExecutionDevice {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::GpuDirectMl => "GPU · DirectML",
        }
    }

    #[must_use]
    pub const fn short_name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::GpuDirectMl => "gpu",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "cpu" | "процессор" => Some(Self::Cpu),
            "gpu" | "directml" | "видеокарта" => Some(Self::GpuDirectMl),
            _ => None,
        }
    }

    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Cpu => Self::GpuDirectMl,
            Self::GpuDirectMl => Self::Cpu,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeviceCapabilityStatus {
    Unknown,
    Ready,
    Unavailable,
}

impl DeviceCapabilityStatus {
    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Unknown => "?",
            Self::Ready => "✓",
            Self::Unavailable => "✗",
        }
    }
}
