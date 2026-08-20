---
tdr_id: "TDR-FS-1.6"
title: "Graph query surface и dtree adapter"
status: "заменено"
implementation_stage: "заменено"
parent_tdr_id: "TDR-FS-1"
child_tdr_ids: []
updated: "2026-08-15"
---
# TDR-FS-1.6 — Graph query surface и dtree adapter

Решение передано FastGraph и заменено его TDR-FG-1.5 и TDR-FG-1.10. GraphProvider публикуется по профилям, а не единым этапом FG7. Dtree потребляет графовый порт FastGraph, а не FastSearch. [Карта передачи](<../FastGraph.md>) содержит целевые ссылки.

## Контекст TDR

- [PAR-FS-005](<../../Paradigms/Архитектура/Графы знаний/04 Полный граф через ограниченные запросы.md>) — full namespace и bounded responses.
- [PAR-FS-001](<../../Paradigms/Архитектура/01 FastSearch как knowledge plane.md>) — dtree boundary.

## Входы и результат

GraphQuery содержит graph revision/current policy, anchors, operation, edge kinds, direction, depth, node/edge/byte/time budget и optional source snippets. GraphResponse содержит ordered nodes/edges, completeness, truncation/continuation, provider status и query provenance.

## Operations

- `neighbors` — hierarchy/relations вокруг anchors;
- `callers` / `callees` — incoming/outgoing CALLS;
- `path` — объяснимый path между nodes;
- `impact` — affected set с root causes;
- `documentation` — code → docs/TDR/paradigms;
- `source` — locator/snippet по budget;
- `search_anchor` — exact/lexical/semantic re-anchor.

## Revision semantics

Query либо закрепляет exact graph/document/code/link revision set, либо явно запрашивает latest. Continuation относится к тому же revision. Если revision больше недоступна, ответ возвращает typed error и доступные recovery options, а не тихо переключается на latest.

## Limits

Depth ограничен configurable bound, response — nodes/edges/bytes/time. При truncation response возвращает frontier и continuation token/next query hint. Один directed edge не дублируется для reverse traversal.

## Dtree integration

FastSearch возвращает domain DTO без assignment/agent state. Dtree связывает response с ContextDelivery, хранит query history и решает, какой result передать agent. FastSearch не требует route handle и не пишет dtree SQLite.

## Ошибки и граничные случаи

- Unknown/stale revision или invalid continuation.
- Anchor removed between latest queries.
- Unsupported relation/language capability.
- Query budget insufficient even for anchor metadata.
- Provider partial/unavailable mid-query.

## Инварианты

- Deterministic ordering на exact revision.
- Full graph remains addressable through multiple bounded queries.
- Truncation explicit.
- Raw graph DB is not public API.
- CLI/MCP/dtree adapters share one query application service.

## Связь с кодом и проверки

DT4 AgentSurface search/get/related/status and production runtime are producer. Required contract tests operations, limits, continuation, exact/latest revision, partial language, CLI/MCP parity and dtree fake adapter.

## Состояние реализации

Механизм отсутствует и относится к DT10.

