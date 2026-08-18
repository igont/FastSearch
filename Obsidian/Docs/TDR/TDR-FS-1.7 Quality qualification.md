---
tdr_id: "TDR-FS-1.7"
title: "Quality qualification"
status: "заменено"
implementation_stage: "заменено"
parent_tdr_id: "TDR-FS-1"
child_tdr_ids: []
updated: "2026-08-15"
---
# TDR-FS-1.7 — Quality qualification

Графовая часть решения передана FastGraph и заменена его TDR-FG-1.7. Квалификация поисковых моделей остаётся в действующих TDR-FS-2.4 и TDR-FS-2.5. [Карта передачи](<../FastGraph.md>) разделяет эти ответственности.

## Контекст TDR

- [PAR-FS-010](<../../Paradigms/Архитектура/Сопровождение графа/05 Evidence-first выбор моделей.md>) — benchmark и model ladder.

## Входы и результат

Входы: versioned repositories/corpora, expected retrieval sets, verified links, expected summaries, controlled change scripts и candidate models. Результат: quality report, qualified capability matrix, routing thresholds и regression dataset.

## Datasets

- Russian documentation with English identifiers/terms.
- Exact/lexical/semantic queries compared with manual reading.
- Known document hierarchy, supersession and authority.
- Known code declarations/edges for each language capability.
- At least 50 verified doc↔code links.
- Controlled changes: formatting, within-scope reorder, rename, body-only algorithm, signature, call add/remove, document/TDR change, dynamic dispatch.

## Metrics

Retrieval precision/recall/ranking; summary factuality/responsibility/input/output/side-effect coverage; hallucination rate; ID preservation; edge accuracy/unresolved rate; false/missed stale propagation; cross-link precision/recall; curator time/cost and accepted-node throughput.

## Qualification

Capability gets per-task threshold and model/provider provenance. Default route выбирается только после passing evidence. Local failure routes to stronger local/Codex; it does not silently lower quality. Model/prompt change invalidates relevant projection qualification.

Interactive comparison использует один admitted corpus generation и один query для всех catalog candidates. Vector top-K каждой модели измеряется независимо; shared exact/lexical baseline материализуется один раз. Raw cosine scores между моделями не сравниваются. Cross-model analysis использует ranks, stable-ID overlap, expected-set metrics, latency и explicit human/agent judgments.

## Errors and bias controls

- Benchmark too close to training/example prompts.
- Only simple functions represented.
- Dynamic-language unresolved cases excluded from denominator.
- Human judgments inconsistent.
- Aggregate score hides severe hallucination class.

Datasets include difficulty/language strata, blind review samples and per-metric thresholds.

## Инварианты

- Experiment is reproducible from exact source/model/prompt revisions.
- Quality gates are task-specific.
- Manual baseline is preserved.
- No single demo qualifies a model.
- Ad hoc comparison помогает исследованию, но смена default требует versioned query suite и сохранённого qualification report.
- DT stage cannot claim full graph quality without corresponding language/change evidence.

## Связь с кодом и проверки

Reuse DT3 quality/performance contract formats where appropriate. Add graph-specific fixtures, expected edges/summaries/links, scripted changes, score computation and regression reports.

## Состояние реализации

Partial retrieval evidence exists; complete graph qualification is absent. Foundation begins in DT5 and reaches continuous curator/quality operation in DT11.
