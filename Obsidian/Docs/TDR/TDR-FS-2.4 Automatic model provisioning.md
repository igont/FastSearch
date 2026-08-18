---
tdr_id: "TDR-FS-2.4"
title: "Automatic model provisioning"
status: "принято"
implementation_stage: "текущее"
parent_tdr_id: "TDR-FS-2"
child_tdr_ids: []
updated: "2026-08-16"
---
# TDR-FS-2.4 — Automatic model provisioning

## Контекст TDR

- [PAR-FS-010](<../../Paradigms/Архитектура/Сопровождение графа/05 Evidence-first выбор моделей.md>) — evidence-first qualification и model provenance.
- [TDR-FS-2](<TDR-FS-2 Workspaces и terminal UX.md>) — один executable, общий machine-local state и отсутствие технических path prompts.
- [Evidence-first выбор моделей](<../../Paradigms/Архитектура/Сопровождение графа/05 Evidence-first выбор моделей.md>) - поисковые критерии смены модели по умолчанию.

## Входы и результат

Входы: выбранный `EmbeddingModelId`, product data directory, catalog repository/revision, runtime contract и optional `HF_ENDPOINT`. Результат: одна активная и готовая machine-local embedding-модель для workspace либо typed `VectorRetrieval` failure без ложного Current state. Несколько моделей могут находиться в cache, но в runtime используется ровно одна.

## Механизм

1. Workspace profile хранит stable model slug; отсутствие поля в старом profile означает backward-compatible `multilingual-e5-small`.
2. Interactive creation показывает framework-rendered каталог; `/model set <номер>` сначала provisions и probes candidate, затем сохраняет выбор. Ошибка до admission не изменяет прежнюю active model.
3. Runtime определяет общий product data directory через `FASTSEARCH_HOME` или platform data directory. Пользователь не вводит model path.
4. Provisioner получает обязательные assets строго из catalog revision в Hugging Face-compatible cache. Каждая сетевая попытка ограничена 60 секундами; до 24 попыток продолжают `.download` через HTTP Range. Только после полной публикации FastEmbed открывает соответствующий ONNX либо Candle runtime из того же cache.
5. Readiness probe выполняет inference на синтетической строке, проверяет finite vector и точную размерность. Corpus, source state и indexes при этом не читаются и не изменяются.
6. Ready marker содержит slug, repository, catalog revision и dimension; повторное открытие не повторяет тяжёлый probe.
7. Смена model identity не переиспользует прежнюю vector projection. `/index update` либо `/index rebuild` остаются отдельным явным действием.
8. Последний search можно зафиксировать через `/experiment record`: journal сохраняет model, query, hit count, latency и judgment в portable `.fastsearch/knowledge`.
9. После успешной CPU readiness provisioner выполняет machine-local GPU probe без доступа к corpus. Состояние `unknown/ready/unavailable`, backend и диагностическая причина сохраняются рядом с model cache и повторно используются для той же catalog revision.
10. Terminal catalog разделяет readiness весов и execution capability: CPU/GPU не являются одним общим статусом. До фактической пробы GPU отображается `?`; успешный finite embedding точной размерности даёт `✓`; доказанная недоступность текущего backend даёт `—`.

Каталог кандидатов: `multilingual-e5-small`, `multilingual-e5-base`, `multilingual-e5-large`, `qwen3-embedding-0.6b`, `nomic-embed-text-v2-moe`. E5 использует mean-pooling и `query:`/`passage:`; Qwen3 использует last-token runtime и retrieval instruction; Nomic v2 MoE использует mean-pooling и `search_query:`/`search_document:`. Подмена этих контрактов единым pooling запрещена.

## Ошибки и граничные случаи

- Network/mirror недоступен или CDN завис: incomplete transport state не публикуется; bounded retry продолжает тот же `.download`, а после исчерпания возвращается typed failure.
- Cache incomplete/corrupt либо веса удалены после прежнего success: `.ready` сверяется с фактическим weight artifact, stale marker снимается, provider initialization повторяет получение и readiness.
- Два процесса запускаются одновременно: download locks и exclusive install lock исключают частичную публикацию.
- Процесс завершается до atomic rename: active revision root не меняется.
- Model загружена, но inference/indexing завершился ошибкой: ошибка распространяется наружу; vector channel не подменяется lexical success.
- Interactive mode сохраняет exact/FTS/maps/symbols при временной недоступности provider и показывает typed error. Direct mode возвращает non-zero typed failure.

## Инварианты

- Обычный пользователь не вводит model path и не запускает отдельную install-команду; indexing остаётся самостоятельным lifecycle action.
- Завершение install не инициирует index, rebuild, scan источников или search.
- GPU capability не выводится из названия модели или наличия видеокарты: `✓` требует реального inference probe на текущем runtime backend.
- Неуспешная GPU probe не отменяет готовую CPU-модель и не блокирует обычный поиск.
- Repository, catalog revision, runtime family и dimension наблюдаемы. Exact immutable manifest доказан для принятого E5 Small qualification contour; кандидаты не становятся новым default до отдельного evidence gate.
- Готовой считается только полностью опубликованная model root.
- Один model cache используется несколькими workspaces и не попадает в Git.
- `--help` и `--version` не инициируют network/model lifecycle.
- Vector projection не объявляется успешной при provider failure.
- Смена model/revision/runtime contract требует новой qualification evidence и invalidates projection provenance.

## Связь с кодом и проверки

- `src/domain/embedding_model.rs` — stable serializable IDs, dimensions и parser.
- `src/application/model_cache.rs` — catalog sources/revisions, cache location, readiness и model identity.
- `src/domain/execution.rs` — execution devices и трёхсостоянный capability contract.
- `src/application/workspace.rs` — persisted selection и structured experiment journal.
- `src/application/console.rs` — terminal-dialogue catalog, `/model set`, progress, degradation и `/experiment record`.
- `src/application/cli.rs` — automatic provisioning direct surface и operator override.
- `src/adapters/vector/verified_provider.rs` — exact minimal manifest и immutable provider admission.
- `examples/batch_benchmark.rs` — воспроизводимое speed/memory измерение batch size на реальном corpus.
- `tests/e5_auto_pipeline.rs` — явная последовательность provisioning → index → search с обязательным `Vector` hit; отдельный CLI UX test запрещает implicit startup index.
- `tests/model_catalog_pipeline.rs` — сетевой acceptance всех пяти catalog entries: download/open/synthetic inference без workspace index.
- `tests/b2_vector_lifecycle.rs` и vector security tests — lifecycle, determinism, mutation/reparse defense и recovery.
- `evidence/model-auto-provisioning.md` — фактический clean-cache smoke принятого E5 Small contour.
- `evidence/model-catalog-provisioning.md` — источники, ревизии и real-runtime acceptance всего selectable catalog.

## Состояние реализации

Selectable catalog и оба runtime family реализованы для Windows-first product runtime. Обычная suite не требует сети. E5 Small сохраняет прежний immutable cache-gated security oracle; E5 DirectML capability проверяется фактическим inference и сохраняется по catalog revision. Base/Large/Qwen/Nomic требуют последовательной real-model qualification на одном versioned corpus. До этого `multilingual-e5-small` остаётся default, а остальные варианты являются experiment candidates. Linux qualification, GPU execution policy для обычного поиска, Candle CUDA для Qwen/Nomic и полная immutable admission кандидатов остаются открытыми gaps.
