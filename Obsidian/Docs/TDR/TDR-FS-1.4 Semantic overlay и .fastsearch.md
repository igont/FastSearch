---
tdr_id: "TDR-FS-1.4"
title: "Semantic overlay в .fastsearch"
status: "заменено"
implementation_stage: "заменено"
parent_tdr_id: "TDR-FS-1"
child_tdr_ids: []
updated: "2026-08-16"
---
# TDR-FS-1.4 — Semantic overlay в `.fastsearch`

Решение передано FastGraph и заменено его TDR-FG-1.4 и этапом FG5. Переносимое графовое знание больше не принадлежит namespace FastSearch. [Карта передачи](<../FastGraph.md>) содержит целевые ссылки.

## Контекст TDR

- [PAR-FS-008](<../../Paradigms/Архитектура/Сопровождение графа/03 Semantic overlay и graph curator.md>) — description candidates и curator.
- [PAR-FS-009](<../../Paradigms/Архитектура/Сопровождение графа/04 Переносимый namespace .fastsearch.md>) — portable/local boundary.
- [TDR-FS-2.2](<TDR-FS-2.2 Namespace .fastsearch и migration.md>) — full workspace layout и migration.

## Входы и результат

Входы: structural nodes, accepted `.fastsearch/knowledge/curated` records, model candidates и dtree GraphCurator review commit. Результат: effective semantic overlay, pending/stale queue, deterministic portable export и accepted baseline update.

## Overlay record

Node description хранит responsibilities, inputs, outputs, side effects, source hash/revision, model/provider, prompt version, author agent, created_at, review state и stale reason. Cross-link record хранит source/target, kind, origin, confidence и verification provenance.

## Storage boundary

- `.fastsearch/knowledge/curated` — accepted nodes/links, Git-versioned deterministic JSONL или measured deterministic shards.
- `.fastsearch/local/knowledge` — graph cache, embeddings, scan/review candidates и revisions; excluded from Git вместе со всей local branch.
- `.fastsearch/workspace.toml` — portable source configuration; semantic records не дублируют roots вне workspace identity.
- Обязательный semantic bootstrap-файл отсутствует.

## Model/curator flow

1. Mechanical fields извлекает analyzer.
2. Qualified local model формирует candidates простых nodes.
3. High-complexity/centrality/uncertainty остаётся pending.
4. Dtree запускает GraphCurator assignment с subgraph и query access.
5. FastSearch валидирует returned schema, scope и graph revision.
6. Review commit принимает unchanged/new description и edge verdicts атомарно.
7. Deterministic export обновляет curated files.

## Migration CFMap и legacy storage

Current `.cfmap.md` records считаются disposable generated baseline. Explicit curated relations могут импортироваться как candidates с provenance. Legacy root `.search` импортируется только после schema validation. `.cfknowledge` и external service root рассматриваются как legacy local state и пересчитываются либо импортируются по TDR-FS-2.2. Ни один legacy источник не удаляется автоматически.

## Ошибки и граничные случаи

- Curated schema неизвестна или records дублируют stable key.
- Source hash не совпадает с graph revision review.
- Model candidate содержит unsupported responsibility claim.
- Curator пишет node/edge вне assignment scope.
- Deterministic export конфликтует с concurrent accepted commit.
- Git ignore ошибочно исключает curated branch.

## Инварианты

- Automatic rebuild не перезаписывает accepted overlay.
- Local cache deletion не теряет curated data.
- Generated candidate не становится accepted без validation/review.
- Current test CFMap не блокирует migration и может быть пересчитан.
- Author-owned source code/docs не изменяются overlay import.
- `.fastsearch/local` и `.fastsearch/knowledge/curated` имеют разные ownership и durability.

## Связь с кодом и проверки

Baseline: `.cfmap` AUTO/CURATED parser, relations, SQLite state и projection provenance. Нужны curated schema/round-trip, deterministic diff, fresh-clone restore, concurrent review commit, model provenance, selective-ignore и legacy migration fixtures.

## Состояние реализации

Механизм semantic overlay отсутствует и относится к DT8. Workspace layout должен быть материализован раньше или одновременно с первым portable export.

