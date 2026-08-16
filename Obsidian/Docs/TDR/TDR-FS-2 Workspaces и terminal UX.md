---
tdr_id: "TDR-FS-2"
title: "Workspaces и terminal UX"
status: "принято"
implementation_stage: "текущее"
parent_tdr_id: ""
child_tdr_ids: ["TDR-FS-2.1", "TDR-FS-2.2", "TDR-FS-2.3", "TDR-FS-2.4", "TDR-FS-2.5"]
updated: "2026-08-16"
---
# TDR-FS-2 — Workspaces и terminal UX

[← TDR](<00 TDR Index.md>)

## Контекст TDR

- [PAR-FS-011](<../../Paradigms/Архитектура/Рабочие области и интерфейс/01 Рабочая область и два контура источников.md>) — workspace и source contours.
- [PAR-FS-012](<../../Paradigms/Архитектура/Рабочие области и интерфейс/02 Terminal-first интерфейс.md>) — human routing и rendering ownership.
- [PAR-FS-009](<../../Paradigms/Архитектура/Сопровождение графа/04 Переносимый namespace .fastsearch.md>) — portable/local storage boundary.

## Решение

Один системный FastSearch executable управляет глобальным каталогом известных рабочих областей. Каждая область хранит portable configuration/curated knowledge и ignored local state в одном `.fastsearch` namespace. Human flow является state machine поверх `terminal-dialogue`, а не последовательностью обязательных технических path prompts.

## Общий поток

1. Startup resolver сопоставляет current directory с известной областью либо открывает recent-workspace screen.
2. Workspace creation принимает один canonical root.
3. Discovery предлагает ноль или несколько roots для двух фиксированных contours: `documentation` и `code`.
4. Admission фиксирует stable root IDs, relative paths, exclusions, analyzer profiles и одну выбранную embedding-модель.
5. Storage manager открывает или создаёт `.fastsearch`, импортирует portable knowledge и проверяет local state.
6. Index lifecycle показывает initial/current/stale/partial/unavailable state до search prompt.
7. Terminal router принимает query, показывает results и выполняет local result actions.

## Стек механизмов

- TDR-FS-2.1 — global catalog, workspace identity, discovery и source admission.
- TDR-FS-2.2 — `.fastsearch` layout, Git boundary, locking и migration.
- TDR-FS-2.3 — terminal state machine, commands и `terminal-dialogue` boundary.
- TDR-FS-2.4 — machine-local model cache, automatic provisioning и переход к vector pipeline.
- TDR-FS-2.5 — persistent model-specific vector indexes и явно включаемый режим сравнения.

## Инварианты пакета

- Один executable обслуживает несколько рабочих областей без per-repository binary copies.
- Workspace имеет не более двух source contours, но каждый contour допускает несколько roots.
- Heavy index state не хранится в global catalog.
- Только `.fastsearch/local/` обязательно исключается из Git; portable configuration и curated knowledge не теряются при cache deletion.
- Human UI не запрашивает service/index/database paths.
- Human UI не запрашивает model path: workspace выбирает один model slug из framework-rendered каталога, а runtime автоматически восстанавливает веса.
- Обычный search не предлагает model selection на каждом запросе; активная модель сохраняется в workspace.
- Многомодельная выдача существует только в явном experiment contour и не меняет active model обычного режима.
- Search не скрывает full rebuild; freshness и indexing transitions наблюдаемы.
- Current DT3 CLI compatibility сохраняется до отдельного migration cutover.

## Состояние реализации

Workspace catalog, `.fastsearch` physical layout, optional multi-root contours, terminal router и selectable automatic embedding provisioning реализованы. Model selection сохраняется в workspace profile; experiment journal находится в portable knowledge. Human search работает через workspace-owned `ProductionRuntime`; direct three-path CLI сохранён для compatibility. Persistent model partitions и comparison state machine приняты в TDR-FS-2.5, но ещё не реализованы. Полная quality qualification четырёх новых candidates остаётся открытым evidence gap TDR-FS-1.7.
