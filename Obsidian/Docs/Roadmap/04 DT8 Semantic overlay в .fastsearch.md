---
id: "fastsearch/docs/roadmap/dt8-semantic-overlay"
title: "DT8. Semantic overlay в .fastsearch"
type: "roadmap"
status: "принято"
updated: "2026-08-16"
---
# DT8. Semantic overlay в `.fastsearch`

[← Roadmap](<00 Roadmap.md>)

## Producer

DT8 использует stable nodes DT6, revision/review commit DT7 и workspace storage contract TDR-FS-2.2. Первый contour допускает manual curator import/export; automatic dtree launch относится к DT11.

## Наблюдаемый результат

Module/class/function nodes получают versioned semantic descriptions. Accepted descriptions и verified links сохраняются в `.fastsearch/knowledge/curated`, переживают rebuild/fresh clone и не смешиваются с `.fastsearch/local`.

## Новые возможности

- Semantic records responsibilities, inputs, outputs, side effects и provenance.
- Candidate generation для простых nodes qualified local model.
- Pending routing high-complexity/centrality/low-confidence nodes.
- Curator review commit validate scope/revision/schema.
- `.fastsearch/knowledge/curated` deterministic portable nodes/links.
- `.fastsearch/local/knowledge` ignored graph/embeddings/candidates.
- Fresh-clone import и structural rebuild merge.
- CFMap и legacy `.search` compatibility import with migration report.
- Stale reason при source/hash/change events без automatic overwrite.

## Внутренние slices

1. Overlay schema/provenance.
2. Local model adapter and candidate validation.
3. Manual curator package/import.
4. Curated format, import/export and deterministic diff.
5. Fresh clone/rebuild/review commit.
6. CFMap/legacy migration and compatibility sunset criteria.

## Не входит

- Automatic Codex/dtree agent launch.
- Cross-graph semantic candidate generation.
- Коммит embeddings/model weights.
- Защита текущих тестовых CFMap или local indexes как невосполнимого authored source.

## Exit gate

- Accepted overlay survives graph rebuild and fresh clone.
- Local cache deletion loses no accepted semantic data.
- Model candidate cannot become accepted without valid review commit.
- Module/class summaries are reviewed as aggregate responsibility, not concatenated functions.
- Deterministic export yields stable Git diff and handles concurrent revision conflicts.
- CFMap accepted links migrate with provenance; disposable AUTO content can be recalculated.
- Selective Git ignore preserves curated branch and excludes `.fastsearch/local`.

## Связи

- [PAR-FS-008](<../../Paradigms/Архитектура/Сопровождение графа/03 Semantic overlay и graph curator.md>)
- [PAR-FS-009](<../../Paradigms/Архитектура/Сопровождение графа/04 Переносимый namespace .fastsearch.md>)
- [TDR-FS-1.4](<../TDR/TDR-FS-1.4 Semantic overlay и .fastsearch.md>)
- [TDR-FS-2.2](<../TDR/TDR-FS-2.2 Namespace .fastsearch и migration.md>)

