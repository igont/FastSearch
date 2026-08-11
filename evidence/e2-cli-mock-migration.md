# E2 — миграция mock-only контуров

## Заменённые baseline targets

| Удалённый target | Сохранённый инвариант | Real/test-only replacement |
|---|---|---|
| `tests/cli_mock_flow.rs` | executable проходит через application flow и печатает наблюдаемый результат | `tests/cli_real_flow.rs`: `init`, `index update/rebuild`, `search`, `get`, `status`, отдельный process recovery |
| `tests/runtime_mock_contracts.rs` | durable state, exact/lexical result, status и unavailable capability не маскируются | `tests/e1_real_runtime.rs`, `tests/cli_real_flow.rs`, `tests/c1_sqlite_state.rs`, `tests/d1_lexical_projection.rs` |
| `tests/agent_facade_mock_flow.rs` | adapter-neutral port/golden oracle не зависит от production facade | `tests/golden_mock_flow.rs`, `tests/contract_ports.rs`, `tests/dt2_contract_oracle.rs` — test-only `ReferenceFixture` |
| `tests/a3_full_source_reconciliation_contract.rs` | complete-scan state transition имеет явный failure/atomicity contract | `tests/c3_sqlite_full_reconciliation.rs` against real SQLite |

## G-09 классификация оставшихся mock matches

- `BackendKind::Mock` и `ReferenceFixture` остаются только в `tests/support/**`, `tests/golden_mock_flow.rs`, `tests/dt2_contract_oracle.rs` и `src/contract_tests.rs` (`#[cfg(test)]`) как adapter-neutral test oracle.
- В `src/**` нет `MockSource`, `MockState`, `MockLexical`, `MockRuntime`, `MockFacade`, `MockSymbols`, `adapters::mock` или `mock-search`.
- `--test-fail-projection` допускается только в `src/main.rs` и `tests/cli_real_flow.rs`: literal final argument `index update`; нет environment/config hook и нет отдельной runtime factory.
