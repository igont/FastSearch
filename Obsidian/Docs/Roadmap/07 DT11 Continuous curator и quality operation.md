---
id: "fastsearch/docs/roadmap/dt11-curator-operation"
title: "DT11. Continuous curator и quality operation"
type: "roadmap"
status: "заменено"
updated: "2026-08-15"
---
# DT11. Continuous curator и quality operation

Этап заменён FastGraph FG8 и управляющим контуром dtree. [Передача графового контура](<../FastGraph.md>) объясняет владельцев результатов.

[← Roadmap](<00 Roadmap.md>)

## Producer

DT11 требует query/commit contracts DT7–DT10 и dtree agent lifecycle R8/R9. FastSearch не получает собственный Codex launcher: integration начинается событием/assignment через dtree.

## Наблюдаемый результат

После graph update FastSearch создаёт bounded stale/pending queue с root causes. Dtree запускает GraphCurator, передаёт affected graph и rules, а FastSearch валидирует review commit, обновляет `.fastsearch/knowledge/curated` и непрерывно измеряет quality/cost.

## Новые возможности

- Machine-readable curator queue prioritized by impact/centrality/confidence.
- Root-cause grouping вместо тысячи independent stale tasks.
- Dtree event/assignment adapter and exact output contract.
- Curator confirms unchanged descriptions or writes revisions/edge verdicts.
- Review commit atomic baseline advancement.
- Periodic local-model qualification and routing threshold updates.
- Quality regression on benchmark corpus after analyzer/model/prompt changes.
- Operational metrics pending age, curator time, accepted/rejected candidates, false cascades and cost.
- Bounded retry/reconciliation with dtree; manual package/import fallback.

## Внутренние slices

1. Curator queue/prioritization and root grouping.
2. Dtree event/assignment protocol.
3. Review package/schema/commit.
4. Model ladder and routing policy.
5. Continuous benchmarks and regression gates.
6. Failure recovery, manual fallback and operational dashboards.

## Не входит

- FastSearch direct Codex process management.
- Automatic code or authored TDR modification.
- Unlimited curator agents or silent quality-threshold lowering.
- Claim that all dynamic relations become statically exact.

## Exit gate

- One local-only root review closes only its derived cascade.
- Contract change leaves required downstream reviews open.
- Invalid/out-of-scope/stale review commit rejected without partial update.
- Dtree loss/retry does not duplicate accepted commit.
- Manual curator export/import completes same queue item.
- Analyzer/model/prompt change runs affected qualification suite.
- Operational report exposes backlog, latency, quality and cost without opaque aggregate score.

## Связи

- [PAR-FS-008](<../../Paradigms/Архитектура/Сопровождение графа/03 Semantic overlay и graph curator.md>)
- [PAR-FS-010](<../../Paradigms/Архитектура/Сопровождение графа/05 Evidence-first выбор моделей.md>)
- [TDR-FS-1.7](<../TDR/TDR-FS-1.7 Quality qualification.md>)
