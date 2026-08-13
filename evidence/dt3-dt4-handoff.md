# FastSearch — handoff DT3 → DT4

Дата проверки: `13.08.2026 10:57`.

## Точный baseline

- Финальная integration revision DT3 до архива: `7d055df6f2aa2eed81ec31dc6de647221e7b2ee2`.
- Archive commit: `77036562e0dabdf9ea255a41b451105ab3e27283`.
- Финальный merge DT3 в `main`: `5b25f5bf235309761f4376dc4143b246c8409c66`.
- Канонический архив: `.agents/archive/DT-12-08-2026_12-25-Семантическая-и-кодовая-навигация-FastSearch.zip`.
- Локальный `main` на момент проверки чист и указывает на точный финальный merge. Remote tracking ref `igont/main` отстаёт на 97 commits, поэтому baseline принят как локальный Git baseline, но пока не как независимо восстановимый remote baseline.

## Повторная проверка

На exact `5b25f5bf235309761f4376dc4143b246c8409c66` выполнены:

- `cargo test --workspace --all-targets --locked --no-fail-fast` — `PASS`; три проверки, требующие `FASTSEARCH_E5_MODEL_ROOT`, явно `ignored`;
- `cargo fmt --check` — `PASS`;
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — `PASS`;
- `cargo build --release --locked` — `PASS`;
- `git diff --check` — `PASS`.

Текущий прогон подтверждает обычный baseline, но не заменяет cache-gated E5 evidence DT3. Локальный model cache не входит в Git и должен быть повторно прочитан и проверен перед vector-dependent acceptance DT4.

## Что принято после DT3

| Область | Фактическое состояние |
|---|---|
| Application boundary | `AgentSurface` задаёт `search/get/related/status/index_status` и уже используется CLI/runtime tests. |
| Production composition | `ProductionRuntime` соединяет filesystem sources, SQLite authority, Tantivy, optional local E5, maps, symbols и deterministic fusion. |
| Configuration | Document, code, service и optional E5 roots передаются снаружи; corpus и model paths не захардкожены. |
| Lifecycle | Durable state и rebuildable projections различаются; stale/degraded/unavailable наблюдаемы. |
| Search output | Ordering, channels, freshness и optional vector provenance существуют в domain result. |
| Agent transport | Реального protocol adapter нет; `Capability::AgentSurface` не объявлена доступной production runtime. |
| Mocks | Production mock route отсутствует; `BackendKind::Mock` и `ReferenceFixture` остаются test-only oracle. |
| Release evidence | DT3 archive содержит accepted integration report; committed `evidence/dt3/e2-release.json` привязан к измеренному product revision, а финальные safety amendments проверены отдельно. |

## Почему цель DT4 остаётся актуальной

FastSearch уже полезен через CLI, но для агента CLI остаётся дорогим и слабым protocol boundary: каждый вызов создаёт процесс, требует machine paths и разбора человекочитаемого текста. Текущий `AgentSurface` доказывает общую application semantics, но не решает transport framing, typed wire schema, bounded output, server lifecycle, cancellation и подключение реального MCP-клиента.

Поэтому DT4 не должно заново строить retrieval core. Его цель — превратить принятый core в безопасный, ограниченный и воспроизводимый локальный agent tool.

## Уточнённый целевой контур DT4

1. Один FastSearch executable получает отдельный локальный MCP server mode. Базовый кандидат — `stdio`; listener, remote access, auth и cloud contour находятся вне текущего scope.
2. MCP adapter вызывает тот же `ProductionRuntime`/`AgentSurface`, что проверяется CLI. Дублировать search, ranking, related или status logic в transport запрещено.
3. Wire DTO являются отдельным versioned protocol contract. Domain structs не становятся wire format автоматически.
4. Semantic parity означает равные records, ordering, channels, provenance, freshness и error classes; byte-to-byte равенство CLI text и MCP JSON не требуется.
5. Базовые read tools — `search`, `get`, `related`, `status`. Ownership mutation-команд решается до implementation: indexing остаётся operator CLI либо публикуется отдельными maintenance tools, но search не выполняет скрытый update/rebuild.
6. Startup один раз принимает document/code/service/model configuration и удерживает долгоживущий runtime. Sequential ownership допустим как стартовый вариант; concurrency добавляется только после измерения и доказанной синхронизации mutable indexing boundary.
7. Protocol boundary ограничивает query length, result count, payload bytes, execution time и одновременную работу. Конкретные числа выбираются после baseline measurement, а не переносятся из предположений.
8. Локальный `stdio` contour не требует credentials. Machine-specific absolute paths, corpus text сверх принятого result contract и model-cache details не возвращаются клиенту.
9. Release остаётся Windows-first: один executable, checksum, locked build, fresh-directory smoke и реальное подключение MCP-клиента. Optional E5 weights остаются внешним immutable local cache.

## Наблюдаемые gaps до materialization

- В crate нет MCP dependency, server entrypoint, handshake smoke и protocol DTO.
- `SearchQuery` пока не выражает limit, pagination, deadline или cancellation.
- `AgentSurface` не содержит index mutations; их ownership нельзя угадывать внутри transport-ветки.
- CLI presentation теряет часть подробного status reason и projection provenance, поэтому parity oracle должен сравнивать domain semantics до rendering.
- `ProductionRuntime::index/rebuild` требуют mutable ownership, а read methods принимают shared reference; долгоживущая process/concurrency model ещё не зафиксирована.
- Startup configuration сейчас positional CLI-only; для MCP нужен однозначный, тестируемый contract без secret/absolute-path leakage.
- Публичный `scaffold_status()` всё ещё возвращает исторический DT1 diagnostic и не отражает production runtime; до выпуска его нужно осознанно удалить, deprecated-изолировать либо оставить как явно legacy API.
- Текущий `main` не отражён remote tracking ref и не является remote-backed recovery point.
- Cache-gated E5 gates не были повторно выполнены в этой проверке.

## Gate начала DT4

DT4 можно материализовать от exact baseline не ниже `5b25f5bf235309761f4376dc4143b246c8409c66`, если одновременно выполнено следующее:

1. local baseline чист, exact revision и ancestry повторно прочитаны, а DT3 archive открывается;
2. обычные tests, formatting, Clippy и release build проходят;
3. minimal MCP `stdio` handshake доказан bounded spike на текущем Rust toolchain, protocol/SDK version зафиксирована;
4. приняты wire DTO, error/status mapping и список tools;
5. принято решение по indexing ownership и long-lived runtime synchronization;
6. измерены исходные payload/latency, после чего назначены числовые resource limits;
7. определено, является ли remote-backed baseline обязательным gate до исполнения либо только до выпуска;
8. E5 cache повторно квалифицирован либо vector-specific acceptance явно оставлена cache-gated без ложного общего `PASS`.

Материализованного плана DT4 пока нет. Этот handoff уточняет вход и намерение, но не разрешает product implementation сам по себе.
