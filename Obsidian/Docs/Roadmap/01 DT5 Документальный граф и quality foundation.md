---
id: "fastsearch/docs/roadmap/dt5-document-graph"
title: "DT5. Документальный граф и quality foundation"
type: "roadmap"
status: "заменено"
updated: "2026-08-15"
---
# DT5. Документальный граф и quality foundation

Этап заменён FastGraph FG2. [Передача графового контура](<../FastGraph.md>) объясняет новую последовательность. DT5 сохраняется как исторический паспорт и не является следующим этапом FastSearch.

[← Roadmap](<00 Roadmap.md>)

## Producer

DT5 начинается только после принятого DT4 agent surface либо bounded offline spike, не изменяющего DT4 scope. Current exact/FTS/vector behavior остаётся regression contract.

## Наблюдаемый результат

FastSearch индексирует authored documentation как типизированный graph, объясняет hierarchy/authority/links и проходит воспроизводимый Russian+English retrieval benchmark против заранее исследованного corpus.

## Новые возможности

- Nodes corpus/document/section/paradigm/TDR/roadmap/contract/registry row/decision/evidence.
- Edges CONTAINS, REFERENCES, SUPERSEDES, IMPLEMENTS, DERIVED_FROM, EVIDENCES и CONFLICTS_WITH.
- Authority/source kind, content hash, document revision и parser completeness.
- Broken-link, duplicate-ID и invalid-supersession diagnostics.
- Graph-aware exact/get/related projection без изменения DT4 wire semantics задним числом.
- Versioned benchmark queries на русском с английскими identifiers.
- Manual expected sets и сравнение exact/FTS/embedding с обычным чтением.

## Внутренние slices

1. Document graph domain types и storage migration.
2. Type/hierarchy extraction из current Markdown/TSV snapshots.
3. Explicit links, supersession и authority.
4. Graph revision/status/readback.
5. Benchmark corpus, judgments, metrics и baseline report.

## Не входит

- Code hierarchy и language resolver.
- Model-generated node summaries.
- Cross-graph document↔code links.
- Dtree GraphProvider integration.

## Exit gate

- Current DT2/DT3 search regression остаётся pass.
- Representative Obsidian corpus даёт deterministic hierarchy и links.
- Duplicate/broken/supersession failures typed и не создают partial graph.
- Partial parser/source coverage честно отражается status.
- Benchmark versioned и воспроизводим; quality report сравнивает exact/FTS/vector/manual expected set.
- Fresh rebuild и incremental update дают эквивалентный document graph.

## Связи

- [PAR-FS-002](<../../Paradigms/Архитектура/Графы знаний/01 Документальный граф.md>)
- [TDR-FS-1.1](<../TDR/TDR-FS-1.1 Документальный граф.md>)
- [TDR-FS-1.7](<../TDR/TDR-FS-1.7 Quality qualification.md>)

