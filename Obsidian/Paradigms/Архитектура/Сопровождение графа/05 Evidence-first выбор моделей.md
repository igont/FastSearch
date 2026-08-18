---
id: "PAR-FS-010"
title: "Evidence-first выбор моделей"
status: "принято"
implementation_stage: "текущее"
tdr_refs: ["TDR-FS-2.4", "TDR-FS-2.5"]
tdr_coverage: "прямое"
updated: "2026-08-15"
---
# Evidence-first выбор моделей

[← Сопровождение графа](<00 Сопровождение графа.md>)

## Статус

Принят обязательный экспериментальный выбор поисковых моделей. Доказательства определяют качество, задержку и допустимый профиль обычного поиска и режима сравнения. Квалификация графовых анализаторов и описаний передана FastGraph.

## Контекст

Качество векторного поиска нельзя назначить по названию модели. Нужны заранее исследованные корпуса, проверенные запросы и ожидаемые результаты, чтобы сравнить индекс с обычным чтением и измерить ложные и пропущенные находки.

## Парадигма

FastSearch сопровождается версионированным эталонным корпусом. Для поиска по документам и коду сравниваются точный, полнотекстовый и векторный режимы с независимо подготовленным ожидаемым набором. Графовые контролируемые изменения и рёбра проверяются в FastGraph.

## Измерения

- retrieval precision/recall и ranking по Russian + English terminology;
- сохранение точных идентификаторов в выдаче;
- устойчивость порядка и полноты результатов;
- задержка и потребление ресурсов каждого режима;
- качество выбора активной поисковой модели.

## Model ladder

Ни одна модель не объявляется вариантом по умолчанию до прохождения поискового контракта качества. Недостаточное качество не компенсируется красивым одиночным примером или молчаливым снижением порога.

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

- [TDR-FS-2.4](<../../../Docs/TDR/TDR-FS-2.4 Automatic model provisioning.md>) — воспроизводимое получение и admission retrieval model.
- [TDR-FS-2.5](<../../../Docs/TDR/TDR-FS-2.5 Режим сравнения embedding-моделей.md>) — model-specific indexes, comparison state machine и presentation contract.
- [Передача FastGraph](<../../../Docs/FastGraph.md>) - квалификация графовых возможностей и семантического слоя.

## Связь с реализацией

DT3 имеет quality/performance contracts и regression queries для retrieval/E5. Текущий runtime предоставляет один selectable slot с E5 Small/Base/Large, Qwen3 Embedding 0.6B и Nomic Embed Text v2 MoE; provisioning не запускает indexing или search. `/experiment record` сохраняет model/query/hit count/latency и human/agent judgment. E5 Small остаётся default с прежним immutable-manifest evidence. Persistent model-specific vector partitions, readiness dashboard и одновременная разделённая выдача ещё не реализованы и являются ближайшим gap TDR-FS-2.5. Новый default выбирается только после одинакового corpus benchmark всех candidates.
