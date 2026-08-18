---
id: "PAR-FS-001"
title: "FastSearch как knowledge plane"
status: "заменено"
implementation_stage: "заменено"
tdr_refs: ["TDR-FS-1"]
tdr_coverage: "прямое"
updated: "2026-08-16"
---
# FastSearch как knowledge plane

Парадигма заменена документом [FastSearch как поисковый контур](<02 FastSearch как поисковый контур.md>). Полный граф передан [FastGraph](<../../Docs/FastGraph.md>). Этот файл сохраняется как история прежней границы.

[← Архитектура](<00 Архитектура.md>)

## Статус

Принята целевая роль поверх существующего retrieval core DT1–DT3 и запланированного agent surface DT4. Полный graph и dtree integration ещё не реализованы.

## Контекст

Код, документация, TDR и парадигмы описывают одну систему, но сегодня индексируются как разрозненные records. Dtree нуждается в воспроизводимом knowledge provider, а FastSearch не должен одновременно становиться workflow-orchestrator и запускать Codex самостоятельно.

## Парадигма

FastSearch владеет производными индексами, единым типизированным document/code knowledge graph, revision comparison, semantic overlay, cross-graph links, freshness и bounded query execution. Dtree владеет assignments, agents, context manifests, reviews и curator lifecycle. Source repository владеет кодом, authored documentation и переносимыми `.fastsearch/workspace.toml` и `.fastsearch/knowledge/curated`.

## Границы

Внутри FastSearch:

- source admission и canonical records;
- lexical/vector retrieval;
- hierarchical document/code graph;
- structural/semantic/cross-graph revisions;
- stale/change queue и graph queries;
- import/export accepted `.fastsearch/knowledge/curated` data.

Вне FastSearch:

- создание, ожидание, compaction и wake-up agents;
- acceptance authored documentation и product decisions;
- lifecycle динамического дерева;
- прямое изменение source code по graph finding.

## Инварианты

- Graph является derived knowledge state и не подменяет source repository.
- Provider status различает empty, stale, partial, unavailable и current.
- Dtree не дублирует graph store, а FastSearch не дублирует orchestration state.
- Проверенная семантика переносима; embeddings и structural caches воспроизводимы.
- Один локальный entrypoint может соединять services, не превращая их в монолит.
- Ровно два source contours сохраняют независимые analyzers и namespaces, но публикуются через один knowledge graph/query surface.

## Связи

- [TDR-FS-1](<../../Docs/TDR/TDR-FS-1 Graph knowledge plane.md>) — общий поток graph knowledge plane.
- [Графы знаний](<Графы знаний/00 Графы знаний.md>) — graph namespaces.
- [Сопровождение графа](<Сопровождение графа/00 Сопровождение графа.md>) — revisions и curator.

## Связь с реализацией

DT3 предоставляет production runtime с SQLite, Tantivy, optional E5, `.cfmap.md`, Rust/Python symbols и fusion. DT4 готовит MCP surface `search/get/related/status`. Hierarchical graph, accepted semantic overlay и dtree curator loop относятся к DT5–DT11.
