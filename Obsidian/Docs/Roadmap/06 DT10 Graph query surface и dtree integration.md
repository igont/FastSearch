---
id: "fastsearch/docs/roadmap/dt10-query-surface"
title: "DT10. Graph query surface и dtree integration"
type: "roadmap"
status: "заменено"
updated: "2026-08-15"
---
# DT10. Graph query surface и dtree integration

Этап заменён FastGraph FG7 и последующей интеграцией dtree R6. [Передача графового контура](<../FastGraph.md>) объясняет владельцев результатов.

[← Roadmap](<00 Roadmap.md>)

## Producer

DT10 требует complete graph contracts DT5–DT9 и принятую DT4 application/protocol composition. Dtree adapter materializes against its own R6 producer gate; FastSearch remains independently usable through CLI/MCP.

## Наблюдаемый результат

Agent получает initial impact subgraph, затем самостоятельно расширяет depth/direction по полному exact graph revision через bounded CLI/MCP/dtree query surface. Truncation, partial coverage и revision changes видимы.

## Новые возможности

- GraphQuery/GraphResponse DTO with exact/latest revision policy.
- Operations neighbors, callers, callees, path, impact, documentation, source and search_anchor.
- Edge kind, direction, depth, nodes/edges/bytes/time budgets.
- Deterministic ordering and continuation/frontier.
- Current/exact graph/document/code/link revision set.
- Capability/partial language/source status per response.
- Shared application service for CLI/MCP/dtree adapters.
- Dtree conformance fixture and provider status mapping.
- Query provenance suitable for ContextDelivery history.

## Внутренние slices

1. Query domain service and storage indexes.
2. Revision pinning/continuation.
3. Operations and path/impact explanations.
4. Limits/cancellation/truncation.
5. CLI/MCP parity.
6. Dtree fake/real adapter conformance.

## Не входит

- Assignment, agent history and context store inside FastSearch.
- Full graph dump into prompt.
- Automatic GraphCurator launch.
- Hidden raw SQLite protocol.

## Exit gate

- Same exact-revision query is deterministic across CLI/MCP.
- Agent expands one/two/three hops and sideways without new root search.
- Incoming/outgoing traversal works from one stored directed edge.
- Truncated response exposes frontier/continuation; stale continuation typed.
- Exact revision never silently switches to latest.
- Partial language/provider status survives adapters.
- Dtree R6 conformance reads graph without duplicating FastSearch storage.

## Связи

- [PAR-FS-005](<../../Paradigms/Архитектура/Графы знаний/04 Полный граф через ограниченные запросы.md>)
- [TDR-FS-1.6](<../TDR/TDR-FS-1.6 Graph query surface и dtree adapter.md>)

