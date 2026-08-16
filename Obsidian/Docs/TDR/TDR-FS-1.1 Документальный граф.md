---
tdr_id: "TDR-FS-1.1"
title: "Документальный граф"
status: "принято"
implementation_stage: "будущее"
parent_tdr_id: "TDR-FS-1"
child_tdr_ids: []
updated: "2026-08-16"
---
# TDR-FS-1.1 — Документальный граф

## Контекст TDR

- [PAR-FS-002](<../../Paradigms/Архитектура/Графы знаний/01 Документальный граф.md>) — hierarchy и authority.

## Входы и результат

Входы: admitted Markdown/TSV sources из нуля или нескольких document roots, frontmatter, headings, stable IDs, explicit links, status/stage и source revision. Результат: единый typed document namespace, authority/completeness metadata per root и immutable graph revision.

## Механизм

Parser создаёт nodes corpus/document/section/paradigm/TDR/roadmap/contract/registry row/decision/evidence. Explicit parent/child, Markdown links, IDs, registry projections и supersession markers создают typed edges. Semantic similarity индексируется отдельно и не меняет explicit authority.

## Логика

1. Reuse current source admission и canonical locators.
2. Classify document type по path/schema/frontmatter с explicit unknown.
3. Создать hierarchy и explicit reference edges.
4. Проверить duplicate IDs, broken links и invalid supersession.
5. Зафиксировать completeness по source kinds/parsers.
6. Опубликовать graph revision и projection provenance.

## Ошибки и граничные случаи

- Unknown document schema или malformed frontmatter.
- Registry row ссылается на отсутствующий document.
- Relative link выходит за admitted root.
- Один stable ID встречается в двух authority sources.
- Generated registry противоречит authored status.

## Инварианты

- Exact lookup сохраняет deterministic behavior current runtime.
- Authority не выводится из vector score.
- Derived row связан с source, но не наследует его нормативность.
- Formatting-only change не меняет stable authored node ID.
- Partial corpus не маркируется complete.
- Stable `root_id` различает одинаковые relative locators из разных document roots.

## Связь с кодом и проверки

Baseline: source scanners/parsers, CanonicalRecord, relations, SQLite generations, Tantivy и E5. Нужны hierarchy fixtures, duplicate/broken/supersession tests, Russian/English query dataset и graph rebuild parity.

## Состояние реализации

Механизм отсутствует и относится к DT5.
