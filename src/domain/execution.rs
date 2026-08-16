//! Runtime execution targets and measured local capabilities.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionDevice {
    Cpu,
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
            Self::Unavailable => "—",
        }
    }
}
