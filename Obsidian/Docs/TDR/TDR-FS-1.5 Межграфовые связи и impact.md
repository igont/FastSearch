---
tdr_id: "TDR-FS-1.5"
title: "Межграфовые связи и impact"
status: "заменено"
implementation_stage: "заменено"
parent_tdr_id: "TDR-FS-1"
child_tdr_ids: []
updated: "2026-08-16"
---
# TDR-FS-1.5 — Межграфовые связи и impact

Решение передано FastGraph и заменено его TDR-FG-1.3, TDR-FG-1.4 и этапом FG6. [Карта передачи](<../FastGraph.md>) содержит целевые ссылки. Файл сохраняется как история прежнего проектирования FastSearch.

## Контекст TDR

- [PAR-FS-004](<../../Paradigms/Архитектура/Графы знаний/03 Межграфовая трассируемость.md>) — relation kinds, candidates и verification.

## Входы и результат

Входы: document/code namespace revisions, explicit links/IDs, semantic descriptions, embeddings и accepted `.fastsearch/knowledge/curated` links. Результат: explicit/verified/candidate cross-namespace edges единого knowledge graph, impact paths и review queue.

## Link production

1. Resolve explicit authored IDs и Markdown/registry relations.
2. Import accepted `.fastsearch/knowledge/curated` links на exact source/target revisions.
3. Search semantic descriptions against document graph и code graph.
4. Emit ranked candidate with explanation/features, not verified edge.
5. Graph curator accepts, rejects or changes kind/scope.
6. Impact query traverses only requested confidence/verification classes.

## Relation model

`DOCUMENTS`, `GOVERNED_BY`, `DECIDED_BY`, `IMPLEMENTS`, `VERIFIED_BY`, `CONTRADICTS`. Edge хранит source/target graph revisions, origin explicit/semantic/imported, confidence, verification, author/reviewer и stale cause.

## Impact

Documentation change starts traversal through verified relations to code nodes and tests; code change traverses to documented/governing sources. Candidate paths are shown separately. Result explains root, path, edge states and truncation boundary.

## Ошибки и граничные случаи

- Semantic top match contradicts explicit relation.
- Document superseded, code node removed или one graph partial.
- Candidate cycle crosses docs/code repeatedly.
- Same source/target has different legitimate relation kinds.
- Review commit refers to stale link revision.

## Инварианты

- Similarity alone never creates verified edge.
- Explicit relation can be stale/conflicting and is not blindly trusted forever.
- Impact result separates verified, candidate и unresolved paths.
- Cross-graph revision changes independently from structural revisions.
- Dtree decides curator timing; FastSearch validates and stores result.

## Связь с кодом и проверки

CanonicalRecord relations and `.cfmap @related` are precursors. Required evidence: known 50 verified doc↔code links, candidate precision/recall, 20 controlled changes, supersession/conflict/cycle and impact explanation tests.

## Состояние реализации

Механизм отсутствует и относится к DT9.
