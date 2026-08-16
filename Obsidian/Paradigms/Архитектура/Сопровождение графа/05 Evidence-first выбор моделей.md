---
id: "PAR-FS-010"
title: "Evidence-first выбор моделей"
status: "принято"
implementation_stage: "текущее"
tdr_refs: ["TDR-FS-1.7", "TDR-FS-2.4", "TDR-FS-2.5"]
tdr_coverage: "прямое"
updated: "2026-08-15"
---
# Evidence-first выбор моделей

[← Сопровождение графа](<00 Сопровождение графа.md>)

## Статус

Принят обязательный experimental selection. Возможность получить полезное описание считается обеспеченной fallback до Codex; evidence выбирает стоимость, latency, routing threshold и допустимый local scope.

## Контекст

Качество embeddings и summaries нельзя назначить по названию модели. Нужны заранее исследованные repositories, verified queries/links и controlled code changes, чтобы сравнить индекс с обычным чтением и измерить ложные graph impacts.

## Парадигма

FastSearch сопровождается versioned benchmark corpus. Для document retrieval сравниваются exact/FTS/embedding modes с ручным expected set. Для code graph выполняется серия контролируемых изменений: formatting, move within scope, rename, body-only algorithm change, signature change, added/removed call, documentation/TDR change и dynamic dispatch.

## Измерения

- retrieval precision/recall и ranking по Russian + English terminology;
- summary factuality, responsibility coverage, inputs/outputs/side effects и hallucination rate;
- stable identity после non-semantic changes;
- edge add/remove accuracy и unresolved coverage;
- false/ missed stale propagation;
- quality cross-graph candidate links;
- curator workload, latency и cost per accepted node.

## Model ladder

Простая local model → более мощная local model → Codex → ручная проверка исключений. Ни одна ступень не объявляется default до прохождения quality contract. Если local models не достигают threshold, routing сразу использует Codex для соответствующего node class.

## Два режима использования

Обычный режим использует ровно одну активную embedding-модель, закреплённую в настройках workspace. Пользователь выбирает её при создании области либо меняет явно; каждый поисковый запрос не повторяет выбор.

Режим сравнения является отдельным временным experiment contour для выбора нового default или admission новой модели. Он проверяет актуальность model-specific vector indexes, предлагает одно явное обновление отсутствующих или stale partitions и после этого выполняет один запрос всеми admitted catalog models. Результаты разделяются по моделям и сохраняют одинаковый top-K contract. Shared lexical baseline показывается один раз; raw scores разных моделей не смешиваются и не ранжируются между собой.

## Инварианты

- Benchmark fixtures и expected judgments versioned.
- Quality threshold определяется по задаче, а не одной aggregate score.
- Model change invalidates соответствующую projection provenance.
- Experiment проверяет полный workflow, а не красивый одиночный summary.
- Выход из режима сравнения не меняет активную модель обычного поиска.
- Model-specific indexes переиспользуются между experiment runs и имеют provenance модели и корпуса.
- Неуспех одной модели отображается как partial comparison и не скрывается удалением её блока.
- Неудача local model не отменяет target capability, но влияет на cost/latency plan.

## Связи

- [TDR-FS-1.7](<../../../Docs/TDR/TDR-FS-1.7 Quality qualification.md>) — datasets, metrics и gates.
- [TDR-FS-2.4](<../../../Docs/TDR/TDR-FS-2.4 Automatic model provisioning.md>) — воспроизводимое получение и admission retrieval model.
- [TDR-FS-2.5](<../../../Docs/TDR/TDR-FS-2.5 Режим сравнения embedding-моделей.md>) — model-specific indexes, comparison state machine и presentation contract.
- [Semantic overlay](<03 Semantic overlay и graph curator.md>) — routing consumer.

## Связь с реализацией

DT3 имеет quality/performance contracts и regression queries для retrieval/E5. Текущий runtime предоставляет один selectable slot с E5 Small/Base/Large, Qwen3 Embedding 0.6B и Nomic Embed Text v2 MoE; provisioning не запускает indexing или search. `/experiment record` сохраняет model/query/hit count/latency и human/agent judgment. E5 Small остаётся default с прежним immutable-manifest evidence. Persistent model-specific vector partitions, readiness dashboard и одновременная разделённая выдача ещё не реализованы и являются ближайшим gap TDR-FS-2.5. Новый default выбирается только после одинакового corpus benchmark всех candidates.
