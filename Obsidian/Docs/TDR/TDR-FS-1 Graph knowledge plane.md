---
tdr_id: "TDR-FS-1"
title: "Graph knowledge plane"
status: "принято"
implementation_stage: "будущее"
parent_tdr_id: ""
child_tdr_ids: ["TDR-FS-1.1", "TDR-FS-1.2", "TDR-FS-1.3", "TDR-FS-1.4", "TDR-FS-1.5", "TDR-FS-1.6", "TDR-FS-1.7"]
updated: "2026-08-16"
---
# TDR-FS-1 — Graph knowledge plane

[← TDR](<00 TDR Index.md>)

## Контекст TDR

- [PAR-FS-001](<../../Paradigms/Архитектура/01 FastSearch как knowledge plane.md>) — ownership FastSearch/dtree/repository.
- [Графы знаний](<../../Paradigms/Архитектура/Графы знаний/00 Графы знаний.md>) — graph namespaces.
- [Сопровождение графа](<../../Paradigms/Архитектура/Сопровождение графа/00 Сопровождение графа.md>) — revisions, overlay и evidence.

## Контекст

DT3 production runtime объединяет records, SQLite, lexical/vector retrieval, `.cfmap` и basic symbols. Целевой этап должен превратить эти источники в два связанных versioned graphs, не сломав exact/FTS contracts и не присвоив FastSearch orchestration agents.

## Решение

Graph knowledge plane строится как отдельные domain contracts поверх текущего retrieval core. Два source contours создают document/code namespaces одного типизированного knowledge graph; каждый contour допускает несколько roots со stable `root_id`. Structural graph и caches остаются производными. Accepted semantic overlay и verified links импортируются из `.fastsearch/knowledge/curated`. Dtree использует provider/query ports и запускает curator agents, FastSearch принимает только validated review commits.

## Общий поток

1. Source adapters создают document/code snapshots для всех admitted roots со stable root IDs и hashes.
2. Language/document analyzers строят structural nodes/edges и completeness.
3. Revision engine сравнивает observed graph с accepted baseline и создаёт ChangeEvents.
4. Semantic pipeline создаёт model candidates и stale queue.
5. `.fastsearch/knowledge/curated` import накладывает accepted descriptions и verified links.
6. Cross-graph resolver создаёт explicit edges и ranked candidates.
7. Query surface обслуживает revision-pinned bounded traversal.
8. Dtree graph curator возвращает validated review commit.
9. FastSearch атомарно принимает overlay/edge verdict и продвигает accepted baseline scope.

## Стек механизмов

- TDR-FS-1.1 — document graph и authority.
- TDR-FS-1.2 — code hierarchy, AST/resolver и identity.
- TDR-FS-1.3 — revisions, deltas и causal invalidation.
- TDR-FS-1.4 — semantic overlay, `.fastsearch/knowledge/curated` и curator commit.
- TDR-FS-1.5 — cross-graph candidates/verified links и impact.
- TDR-FS-1.6 — graph queries и dtree boundary.
- TDR-FS-1.7 — benchmark datasets, metrics и model qualification.

## Инварианты пакета

- Current source code/docs остаются authority structural facts.
- Graph revision, document revision, code revision и link revision различаются.
- Rebuild не перезаписывает accepted semantic content.
- Partial language/document coverage видима.
- Semantic similarity создаёт candidate, но не verified fact.
- FastSearch не создаёт Codex agents и не меняет dtree lifecycle.
- `.cfmap` допускается только как migration/compatibility source после принятия target.
- Source contour остаётся `documentation` или `code`; multiple roots не создают новые graph namespaces.

## Состояние реализации

Current baseline предоставляет необходимые record/retrieval/storage precursors, но каждый graph mechanism остаётся future. DT4 не расширяется этими capabilities; DT5–DT11 материализуются последовательно.
