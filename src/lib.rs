//! Общий core FastSearch.
//!
//! На этом этапе crate намеренно не предоставляет поисковые возможности.

pub mod domain;
pub mod ports;

/// Возвращает наблюдаемый статус минимального каркаса приложения.
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
