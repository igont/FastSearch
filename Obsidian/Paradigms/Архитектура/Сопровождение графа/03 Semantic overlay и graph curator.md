---
id: "PAR-FS-008"
title: "Semantic overlay и graph curator"
status: "заменено"
implementation_stage: "заменено"
tdr_refs: ["TDR-FS-1.4"]
tdr_coverage: "прямое"
updated: "2026-08-16"
---
# Semantic overlay и graph curator

Парадигма передана FastGraph и заменена его контрактом проверяемой семантики и этапами FG6 и FG8. Dtree сохраняет управление куратором. [Карта передачи](<../../../Docs/FastGraph.md>) содержит целевые источники.

[← Сопровождение графа](<00 Сопровождение графа.md>)

## Статус

Принята многоуровневая семантика с model routing и проверкой сложных nodes через Codex-agent, запускаемый dtree.

## Контекст

Structural graph отвечает, где symbol и с чем связан, но не объясняет его ответственность. Одинаковая дорогая модель для каждой маленькой функции неэффективна, а автоматический overwrite проверенного class/module description уничтожает полезное знание.

## Парадигма

Каждый module/class/function node может иметь semantic description. Function summary описывает назначение, inputs, outputs, side effects и отдельные responsibilities. Class/module summary агрегирует границы и обязанности, а не склеивает function texts механически.

FastSearch создаёт pending/stale queue и предоставляет graph operations. Dtree создаёт GraphCurator assignment, передаёт affected subgraph, rules и output contract. Curator подтверждает прежнее описание либо создаёт новую accepted revision и scope verdict.

## Model routing

- Малые простые functions направляются дешёвой локальной модели после qualification.
- Большие, многозадачные, high-centrality и low-confidence nodes остаются pending для Codex.
- Несколько responsibilities описываются отдельными фразами.
- Механические signature/inputs/outputs извлекаются analyzer и не поручаются модели без причины.
- Model output является generated candidate до validation/review.

## Provenance

Overlay хранит source hash/revision, model/provider, prompt version, author agent, created_at, review state, stale reason и accepted graph revision. Automatic rebuild не перезаписывает accepted content.

## Инварианты

- FastSearch не запускает Codex напрямую; orchestration принадлежит dtree.
- Curator получает всю доступную graph query surface, но bounded initial context.
- Stale flag снимается только valid review commit.
- Unchanged description может быть повторно accepted без перефразирования ради активности.
- Авторский document/TDR text не редактируется как побочный эффект graph curation.

## Связи

- [TDR-FS-1.4](<../../../Docs/TDR/TDR-FS-1.4 Semantic overlay и .fastsearch.md>) — overlay schema и review commit.
- [Переносимый namespace .fastsearch](<04 Переносимый namespace .fastsearch.md>) — durable accepted layer и local boundary.

## Связь с реализацией

Current symbol records содержат минимальное language/kind/name content, а CFMap CURATED/AUTO является тестовым predecessor. Persistent per-node descriptions, curator queue и review commit отсутствуют.
