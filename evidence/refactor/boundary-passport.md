# Паспорт границ рефакторинга FastSearch v1

Дата: `13.08.2026 14:28`.

## Носитель и область доказательства

- Candidate base: `b083eae73a8963df2612abd3cf44b095f6d8e4f2`.
- Это behavior-preserving refactor passport: он не разрешает изменение публичного Rust API, CLI grammar, JSON schema v1, stdout/stderr, exit codes, persisted-state schema, ranking, lifecycle, security policy или DT2 compatibility.
- SQLite содержит durable canonical authority. Tantivy lexical и optional E5 vector — rebuildable projections; их состояние не является источником истины.
- Windows ordinary contour доказан ниже. Linux/non-Windows runtime и cfg остаются непроверенными; Windows `cargo clippy --all-targets` не является Linux coverage. E5 mutation/provider contour требует accepted immutable `FASTSEARCH_E5_MODEL_ROOT` и не был запущен.

## Exact baseline gates

| Gate | Exact команда | Наблюдаемый итог |
|---|---|---|
| RF-G00 | `git status --porcelain=v1`; `git rev-parse HEAD`; `git merge-base --is-ancestor b083eae73a8963df2612abd3cf44b095f6d8e4f2 HEAD` | clean, `HEAD` равен exact base, ancestry true |
| RF-G01 | `cargo fmt --check` | PASS |
| RF-G02 | `cargo clippy --locked --all-targets -- -D warnings` | PASS |
| RF-G03 | `cargo test --locked` | 91 passed, 0 failed, 4 ignored E5-gated |
| RF-G04 | `cargo test --locked --test a2_public_contract --test contract_ports --test golden_mock_flow` | 10 passed, 0 failed |

Первая параллельная попытка этих команд была прекращена внешним timeout во время contention shared Cargo cache/build directory и дала BrokenPipe. Она не является evidence. Все строки таблицы получены последующим последовательным запуском на exact base.

## Public Rust и ownership boundaries

| Boundary | Наблюдаемый контракт, который сохраняется | Authority / запрещённый drift |
|---|---|---|
| Domain | `CanonicalRecord`, `StableId`, `SourceSnapshot`, `SearchQuery`, `SearchResponse`, lifecycle/capability values сохраняют identity, locator, hashes, ordering, channel/provenance и freshness. | Не добавлять public compatibility break или произвольный wire DTO. |
| Ports | `SourcePort`, `StateStore`, `LexicalRetrieval`, `VectorRetrieval`, `CodeMapPort`, `SymbolPort`, `AgentSurface` сохраняют текущие methods и object-safe boundary. | Новый port/public coordinator требует backflow в A и material replan. |
| Application | `ProductionRuntime` остаётся composition facade; `RealRuntime` остаётся public DT2 compatibility API. `AgentSurface` сохраняет `search/get/related/status/index_status`. | CLI dispatcher не становится application/MCP boundary; legacy contour не удаляется. |
| State and projections | Complete valid source snapshots предшествуют одному SQLite durable transition; projections обновляются только после него. | Никакого cross-store rollback; lexical failure возвращает current typed recovery/degraded outcome; vector failure не откатывает SQLite/lexical и не создаёт ложный `Current`. |
| Adapters | Filesystem admission remains contained, deterministic and fail-closed; maps/symbols retain provenance. Tantivy exact/FTS and E5 fallback retain ranking/freshness. | Parser never publishes partial snapshot. Model bytes must stay verified/pinned for D2. |

## CLI contract matrix

| Surface | Accepted behaviour |
|---|---|
| Direct production | `init`, `index update`, `index rebuild`, `search`, `get`, `related`, `status` accept document, code, service and optional E5 root in the documented order. Search modes are `balanced`, `current`, `design`. |
| Direct DT2 | Document-only `init/index update/index rebuild/search/get/status` stay accepted but unadvertised. |
| Options | `--json` is position-independent before `--`, idempotent, and after `--` is literal data. `--test-fail-projection` remains literal final-update-only test hook and absent from ordinary help. |
| Rendering and process | Technical success goes stdout; error goes stderr. JSON schema remains version `1`; JSON never changes stream separation or exit code. Exit `0` success, `1` runtime failure, `2` usage. |
| Chat | No arguments and `chat` enter human session; commands preserve context, error recovery, `json` toggle, `help`, `exit`/`quit`/`выход`, EOF/cancel and NO_COLOR/redirected ANSI suppression. |

## Scenario matrix

| Scenario | Existing characterization evidence | Required invariant |
|---|---|---|
| Public/domain and golden output | `a2_public_contract`, `contract_ports`, `golden_mock_flow` | public port shape, stable identity, provenance, stale legacy state and goldens do not drift. |
| Direct/chat grammar | `cli_ux`, `cli_real_flow` | command acceptance, errors, stdout/stderr, JSON and exit codes remain deterministic. |
| Production containment | `e2_production_runtime`, `e2_release_runner` | overlap, junction/reparse, parent swap, run ownership and cleanup fail before external write/delete. |
| SQLite and lexical lifecycle | `c1_sqlite_state`, `c2_sqlite_lifecycle`, `c3_sqlite_full_reconciliation`, `d1_lexical_projection`, `e1_real_runtime` | complete validation before authority mutation; reopen/rebuild/failure freshness stays truthful. |
| Fusion/maps/symbols | `e1_fusion`, `c2_related_navigation`, `d1_tree_sitter_identity`, `d2_symbol_lifecycle` | channel calibration, exact dominance, source ordering, related provenance and symbol lifecycle remain unchanged. |
| Source admission | source unit tests, `c1_cfmap_lifecycle`, `a2_public_contract`, `dt2_contract_oracle` | contained UTF-8 allowed files only; junction, malformed Markdown/TSV, duplicate identity and partial snapshot are rejected. |
| E5 provider | ordinary `b2_vector_lifecycle`; RF-G10 ignored tests | missing provider is typed/not Current; accepted cache mutation/replacement/reparse/race keeps authority and prevents false hits. |
| DT2 compatibility | `e1_real_runtime`, `cli_real_flow`, `dt2_contract_oracle` | document-only commands and `RealRuntime` recover/rebuild as before. |

## Executable lifecycle for every consumer leaf

The listed RED command is executed by the corresponding code-worker on its leaf base before product change. A1 records the oracle; it does not add speculative failing tests.

| Leaf | Causal RED and focused control on leaf base | Minimal GREEN revision | Separate REFACTOR revision and final evidence |
|---|---|---|---|
| B1 | Add security-owner architecture assertion: facade may delegate, but path/run/Win32 implementation must live in dedicated internal owner. It must RED on A base while `cargo test --locked --test e2_production_runtime --test e2_release_runner` is GREEN. | First B1 candidate makes assertion and focused security tests GREEN without public/error/storage change. | Next candidate removes temporary re-export/duplicate path logic; audit import direction; RF-G12 plus B1 tests; then RF-G00, RF-G01..G05, RF-G13L. |
| B2 | Add coordinator assertion that `ProductionRuntime` does not orchestrate multiple adapters directly; exercise full-snapshot validation, one SQLite transition, lexical failure and optional-vector failure without rollback. It REDs on accepted B1 base while RF-G04..G06 and `e1_fusion`, `c3_sqlite_full_reconciliation`, `c2_related_navigation`, `production_mock_audit` are GREEN. | First B2 coordinator candidate makes assertion and focused outcomes GREEN. | Next candidate removes forwarding/duplicate flow; ownership audit + RF-G12; final RF-G00, RF-G04..G06, RF-G13L with explicit two failure outcomes. |
| B3 | Add compatibility-owner assertion: public `RealRuntime` is exported from a dedicated internal compatibility module. It REDs on B2 base while `e1_real_runtime`, `cli_real_flow`, `dt2_contract_oracle`, RF-G04/RF-G07 are GREEN. | First B3 candidate makes assertion and legacy outputs GREEN. | Next candidate removes duplicate lifecycle/temporary re-export beyond required API; RF-G12 plus B3 controls; RF-G13L then B-branch RF-G00, RF-G04..G07, RF-G13B and master review. |
| C1 | Add private typed-command parser/dispatcher tests. They must fail because target boundary is absent on A base while `cli_ux`, `cli_real_flow`, RF-G04 are GREEN. | First C1 candidate passes typed parsing and direct/legacy process controls with identical grammar/output. | Next candidate removes positional-vector reconstruction/duplicate parsing; assert CLI-only private visibility and no MCP/domain DTO; RF-G12 then RF-G00, RF-G04, RF-G07, RF-G13L. |
| C2 | Add presenter and typed-console seam tests, absent on accepted C1 base; current CLI/golden controls remain GREEN. | First C2 candidate makes seam and `cli_ux`, `cli_real_flow`, `golden_mock_flow` GREEN, including scripted help/version/cancel/continue. | Next candidate removes duplicate mappings and argument-vector translation; prove presenters do no runtime/filesystem work; RF-G12; RF-G13L then C-branch RF-G00, RF-G04, RF-G07, RF-G13B and master review. |
| D1 | Add parser-without-traversal/import-direction oracle: format parser is callable with bounded content/locator and scanner owns discovery. It REDs on A base while current source/admission suite is GREEN. | First D1 candidate makes boundary and RF-G04/RF-G08 plus `dt2_contract_oracle`, `a2_public_contract` GREEN. | Next candidate deletes duplicate grammar/traversal forwarding and proves no parser filesystem discovery; RF-G12; RF-G00, RF-G01..G04, RF-G08, RF-G13L. |
| D2 | After bounded immutable-cache preflight, add verified-provider/projection ownership assertion: lifecycle cannot bypass verified bytes and provider/Win32 verification is absent from lifecycle. It REDs on D1 base while ordinary fallback is GREEN. | First D2 candidate passes ordinary vector tests and all RF-G10 ignored provider/security tests. | Next candidate removes duplicate verified-load/provider path; prove no lifecycle bypass; RF-G12 with all RF-G10 repeats; RF-G13L then D-branch RF-G00, RF-G01..G03, RF-G08..G10, RF-G13B and master review. |

## E5 cache-gated preflight for D2

Before D2 RED, the worker records only cache identity/manifest outcome, never a machine path or source text. `FASTSEARCH_E5_MODEL_ROOT` must be supplied, immutable and complete; it is then used for:

```text
cargo test --locked --test b2_vector_lifecycle -- --ignored
cargo test --locked adapters::vector::security_tests -- --ignored
cargo test --locked --test e2_production_runtime configured_provider_failure_preserves_authority_then_recovers_without_false_hits -- --ignored
```

Unavailable cache blocks D2 and final acceptance; it is not waived by ordinary fallback PASS.

## Explicit exclusions

- No MCP/DT4 transport, new public port, dependency, protocol DTO, storage migration or ranking redesign.
- No Linux qualification is implied by this Windows evidence.
- No replacement of existing characterization tests by structure-only checks: every architecture oracle has a simultaneous observable control.
