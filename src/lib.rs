//! Общий core FastSearch: domain contracts, adapters и application compositions.

pub mod adapters;
pub mod application;
pub mod domain;
pub mod ports;

/// Исторический DT1 diagnostic, сохранённый до release-инвентаризации публичной поверхности.
///
/// Он не описывает состояние [`application::ProductionRuntime`]; production callers должны
/// использовать `AgentSurface::status` и `AgentSurface::index_status`.
#[must_use]
pub const fn scaffold_status() -> &'static str {
    "FastSearch scaffold: search capability is not configured."
}

#[cfg(test)]
mod tests {
    use super::scaffold_status;

    #[test]
    fn reports_that_search_is_not_configured() {
        assert_eq!(
            scaffold_status(),
            "FastSearch scaffold: search capability is not configured."
        );
    }
}

#[cfg(test)]
mod contract_tests;
