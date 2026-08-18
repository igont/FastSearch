---
tdr_id: "TDR-FS-1.3"
title: "Ревизии и причинная актуальность графа"
status: "заменено"
implementation_stage: "заменено"
parent_tdr_id: "TDR-FS-1"
child_tdr_ids: []
updated: "2026-08-15"
---
# TDR-FS-1.3 — Ревизии и причинная актуальность графа

Решение передано FastGraph и заменено его TDR-FG-1.3 и этапом FG4. [Карта передачи](<../FastGraph.md>) содержит целевые ссылки. Файл сохраняется как история прежнего проектирования FastSearch.

## Контекст TDR

- [PAR-FS-007](<../../Paradigms/Архитектура/Сопровождение графа/02 Ревизии рёбер и причинная актуальность.md>) — edge states, change levels и root causes.

## Входы и результат

Входы: previous observed revision, new structural graph, accepted semantic baseline и change classification. Результат: immutable graph revision, edge/node deltas, ChangeEvents, derived affected sets и curator review queue.

## State model

Edge хранит presence, delta, semantic state, confidence и cause отдельно. Structural current graph определяет present/removed facts. Semantic current/updated/stale/unverified сравнивается с accepted baseline. Node summary вычисляется из incident edges и root events.

## Change classification

Normalized AST hashes разделяют formatting (L0), body-only implementation (L1), behavior/effect candidate (L2), contract/signature (L3) и architectural/module boundary (L4). Algorithm даёт initial classification; high-impact/uncertain result требует curator verdict.

## Causal propagation

ChangeEvent хранит root node, revision, level и rule. Traversal строит affected set лениво с SCC condensation для cycles. Curator принимает `local_only`, `bounded_to`, `propagate_to_callers` или `architectural_review_required`. Закрытие root event снимает только derived states, доказанно зависящие от него.

## Review commit

Accepted baseline продвигается атомарно для exact scope nodes/edges и graph revision. Removed edges остаются tombstones до retention/compaction. Concurrent commit к stale graph revision отклоняется.

## Ошибки и граничные случаи

- Low-confidence classification пытается закрыть большой cascade.
- Multiple root causes затрагивают одно edge.
- Cycle пересекает module/document boundary.
- Structural graph partial, поэтому absence не доказывает removal.
- Curator commit основан на superseded graph revision.

## Инварианты

- L1 local implementation update не создаёт автоматический downstream stale cascade.
- Removed relation не маскируется `stale`.
- Один edge может зависеть от нескольких unresolved root events.
- Node summary воспроизводим и не редактируется вручную.
- Accepted baseline не переписывает historical observed revisions.

## Связь с кодом и проверки

Current StateChangeSet и SQLite generations дают lifecycle precursor. Нужны revision schema, graph diff, SCC propagation, multi-cause close, concurrent review commit и controlled change benchmark.

## Состояние реализации

Механизм отсутствует и относится к DT7.

