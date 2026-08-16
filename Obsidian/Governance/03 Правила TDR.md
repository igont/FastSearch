---
id: "fastsearch/governance/tdr-rules"
title: "Правила TDR"
type: "rule"
status: "принято"
updated: "2026-08-15"
---
# Правила TDR

[← Governance](<00 Governance.md>)

## Назначение

TDR фиксирует технический механизм, поддерживающий принятую парадигму. TDR не выдаёт будущий graph contract за текущую реализацию.

## Иерархия

- `TDR-FS-X` — пакет механизмов.
- `TDR-FS-X.Y` — один механизм пакета.
- ID стабилен и не переиспользуется.

## Frontmatter

    ---
    tdr_id: "TDR-FS-X.Y"
    title: "Название"
    status: "черновик | принято | заменено"
    implementation_stage: "текущее | запланированное | будущее | условное"
    parent_tdr_id: "TDR-FS-X"
    child_tdr_ids: []
    updated: "YYYY-MM-DD"
    ---

## Минимальная содержательность

Пакет описывает общий поток, стек механизмов, invariants и состояние реализации. Механизм описывает inputs/results, state before/after, algorithm boundary, failure/conflict modes, tests/evidence и implementation gap.

## Реестр

[TDR Registry.tsv](<../Docs/TDR/TDR Registry.tsv>) является навигационной проекцией. ID, status, stage, parent и path синхронизируются в той же правке.

