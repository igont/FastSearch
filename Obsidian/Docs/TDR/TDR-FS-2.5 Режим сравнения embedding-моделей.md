---
tdr_id: "TDR-FS-2.5"
title: "Режим сравнения embedding-моделей"
status: "принято"
implementation_stage: "частичное"
parent_tdr_id: "TDR-FS-2"
child_tdr_ids: []
updated: "2026-08-16"
---
# TDR-FS-2.5 — Режим сравнения embedding-моделей

## Контекст TDR

- [PAR-FS-010](<../../Paradigms/Архитектура/Сопровождение графа/05 Evidence-first выбор моделей.md>) — evidence-first model selection и разделение обычного/экспериментального режимов.
- [TDR-FS-1.7](<TDR-FS-1.7 Quality qualification.md>) — datasets, metrics и default-model gate.
- [TDR-FS-2.2](<TDR-FS-2.2 Namespace .fastsearch и migration.md>) — shared workspace state и local model partitions.
- [TDR-FS-2.3](<TDR-FS-2.3 Terminal routing и terminal-dialogue.md>) — state machine и rendering boundary.
- [TDR-FS-2.4](<TDR-FS-2.4 Automatic model provisioning.md>) — catalog identities, weights и runtime readiness.

## Входы и результат

Входы: admitted workspace, один canonical corpus generation, catalog моделей, model-specific partition manifests, единый query и одинаковый top-K. Результат: разделённая comparison document с lexical baseline, блоком каждой модели, freshness/provenance, latency и partial failures; active model обычного режима остаётся неизменной.

## Два режима

### Обычный режим

Workspace хранит одну active embedding-модель в `workspace.toml`. Обычный search использует её без повторного выбора. Изменение active model является явной настройкой и не происходит при входе или выходе из comparison mode.

### Режим сравнения

Режим включается явным пользовательским действием и предназначен только для выбора нового default либо admission новой модели. Он не является ежедневным search surface.

```text
обычный поиск
└── active model → fused result

сравнение
├── readiness всех admitted models
├── update stale/absent partitions по подтверждению
├── один query
├── один shared lexical baseline
└── независимый vector top-K каждой модели
```

## Model-specific indexes

Shared SQLite state, source graph и lexical projection не дублируются. Каждая vector projection хранится отдельно:

```text
.fastsearch/local/index/vector/<model-slug>/<model-revision>/
  manifest.toml
  vectors.bin
  records.sqlite
```

Manifest содержит model repository/revision, runtime/pooling/prefix contract, dimension, corpus fingerprint, canonical generation, record count, длительность последнего успешного build и format version. Точный размер вычисляется по committed `manifest.toml + records.sqlite + vectors.bin`; shared state, model weights, lock и temporary files в него не входят. Partition считается `CURRENT` только при полном совпадении provenance; иначе она `STALE`, `ABSENT`, `BUILDING` или `UNAVAILABLE`.

## State machine

1. Вход в comparison mode не меняет active model.
2. Readiness screen проверяет weights/runtime и index partition каждой admitted модели.
3. Если все partitions current, query prompt доступен сразу.
4. При stale/absent partitions primary action предлагает обновить все требуемые модели. Preview показывает модели, corpus generation, приблизительную работу и disk delta; действие требует подтверждения.
5. Обновление один раз reconciles shared corpus/lexical projection, затем выполняется model-by-model строго последовательно в стабильном catalog order. Active model не получает скрытого первого build. Уже current partitions сохраняются; interruption не повреждает опубликованные partitions.
6. Единый query кодируется каждой готовой моделью и выполняется против её partition с одинаковым top-K.
7. Presentation показывает lexical baseline один раз и отдельные блоки моделей в стабильном catalog order.
8. Выход возвращает обычный режим с прежней active model.

## Presentation contract

- Model blocks не fusion-ятся между собой.
- Raw vector scores разных моделей не сравниваются и не образуют общий ranking.
- Результаты адресуются составным номером (`A1`, `B1`) и содержат stable ID, locator, rank, model revision и latency.
- Readiness row каждой модели явно показывает `ИНДЕКС ГОТОВ/НЕ ГОТОВ`, committed size и last successful build duration; отсутствие измерения обозначается `—`, а не нулём.
- Cross-model summary может показывать stable-ID overlap, rank agreement и expected-set metrics, но не объявляет автоматического победителя без judgments.
- Failure одной модели создаёт явно помеченный partial comparison; её блок не исчезает.
- Узкий terminal использует последовательные вертикальные блоки, а не многоколоночную таблицу.
- Подтверждённый update показывает один динамически обновляемый task list: shared corpus и каждая catalog model сохраняют стабильную строку, lifecycle state, текущий этап и фактическую progress bar без процента. При redirected output snapshots дописываются plain text без ANSI cursor control.

## Experiment evidence

Ad hoc query можно оценить непосредственно после выдачи. Для выбора default используется versioned suite с одинаковыми queries, expected sets и judgments. Portable definitions/judgments/summaries находятся в `.fastsearch/knowledge/experiments`; объёмные воспроизводимые run artifacts находятся в ignored `.fastsearch/local/experiments/runs`.

Минимальная запись run содержит query/suite ID, corpus fingerprint, model identity, top-K stable IDs/ranks, query-embedding latency, retrieval latency, failure state и judgment reference.

## Ошибки и граничные случаи

- Corpus изменился между readiness и query: comparison отменяется как stale; модели разных generations не сравниваются.
- Update одной модели неуспешен: уже current partitions сохраняются, экран предлагает retry либо partial run.
- Новая catalog model не имеет partition: она видима как `ABSENT`, а не исключается молча.
- Revision или runtime contract изменился: создаётся новый partition; прежний не выдаётся за current.
- Недостаточно disk space: preview запрещает build до mutation и показывает required/reclaimable size.
- Параллельный writer: partition lock не допускает частичную публикацию.

## Инварианты

- Обычный режим всегда имеет одну явно сохранённую active model.
- Comparison mode не изменяет настройку обычного режима.
- Все сравниваемые модели используют один corpus fingerprint/generation и одинаковый top-K.
- Индекс одной модели не перезаписывает индекс другой.
- Shared state и lexical projection не дублируются по моделям.
- Скачивание weights, построение partition и выполнение query являются разными наблюдаемыми lifecycle actions.
- Default model не меняется по одному ad hoc query.

## Связь с кодом и проверки

Требуются persistent vector store, partition manifest/admission, multi-model index coordinator, comparison session/router и terminal-dialogue result document. Tests должны покрыть current/stale/absent matrix, update confirmation, interruption/resume, corpus race, partial provider failure, unchanged active model, stable presentation order, narrow terminal, `NO_COLOR` и запрет manual renderer output.

Acceptance experiment строит минимум две partitions, перезапускает FastSearch, доказывает повторное открытие без re-embedding, выполняет один query обеими моделями и сохраняет сравнимый run artifact.

## Состояние реализации

Реализованы revision-scoped persistent partitions (`manifest.toml`, `records.sqlite`, `vectors.bin`), admission по model/runtime/corpus contract, повторное открытие без re-embedding, read-only readiness всего catalog, подтверждённый `/update`, динамический task list с progress events для shared indexing, model weights и model partitions, partial model failures, единый lexical baseline, стабильные вертикальные model blocks и `/open A1|L1`. Обычный active model хранится отдельно и comparison router её не меняет. Весь human output проходит через typed documents `terminal-dialogue`; live repaint автоматически деградирует в plain-text snapshots вне terminal.

Стадия остаётся `частичное`: storage round-trip и terminal routing автоматизированы, но обязательный real-model acceptance минимум двух partitions через restart ещё не материализован; update preview пока не выполняет точный disk-space preflight; versioned full comparison run и judgment UI ещё не сохраняются. Эти пункты являются qualification gate перед выбором нового default, а не скрытой частью готовой команды.

Проверки и точные незакрытые gates зафиксированы в [`evidence/model-comparison.md`](../../../evidence/model-comparison.md).
