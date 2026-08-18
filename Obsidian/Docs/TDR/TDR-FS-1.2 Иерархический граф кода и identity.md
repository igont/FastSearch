---
tdr_id: "TDR-FS-1.2"
title: "Иерархический граф кода и identity"
status: "заменено"
implementation_stage: "заменено"
parent_tdr_id: "TDR-FS-1"
child_tdr_ids: []
updated: "2026-08-16"
---
# TDR-FS-1.2 — Иерархический граф кода и identity

Решение передано FastGraph и заменено его TDR-FG-1.2 и этапом FG3. [Карта передачи](<../FastGraph.md>) содержит целевые ссылки. Файл сохраняется как история прежнего проектирования FastSearch.

## Контекст TDR

- [PAR-FS-003](<../../Paradigms/Архитектура/Графы знаний/02 Иерархический граф кода.md>) — node/edge hierarchy.
- [PAR-FS-006](<../../Paradigms/Архитектура/Сопровождение графа/01 Стабильная синтаксическая identity.md>) — simple stable identity boundary.

## Входы и результат

Входы: ноль или несколько admitted code roots, file snapshots, language configuration и previous graph revision. Результат: единый typed code namespace, directed structural edges, per-root/per-language completeness и stable IDs independent of formatting/byte position.

## Identity

Canonical symbol key:

```text
stable_root_id / relative_module / containing_scopes / symbol_kind / name / canonical_signature
```

Body, whitespace, comments, line, byte offset и declaration order не участвуют. Rename и signature change могут дать `removed+added`. Отдельного понятия переноса функции между modules и алгоритма cross-module lineage matching нет: если такой случай фактически возник, прежний symbol исчезает, а новый рассчитывается заново.

## Language adapter

Adapter объединяет Tree-sitter AST, symbol table, imports/namespaces и resolver. Он возвращает exact, inferred или unresolved relation confidence. Language capability имеет отдельные flags declarations/hierarchy/imports/calls/types/inheritance/tests.

## Edge model

`CONTAINS`, `IMPORTS`, `CALLS`, `IMPLEMENTS`, `INHERITS`, `READS`, `WRITES`, `PROVIDES`, `CONSUMES`, `TESTS`. Edge хранится directed один раз и индексируется по source/target.

## Ошибки и граничные случаи

- Dynamic dispatch или reflection не разрешены статически.
- Macro/generated source имеет отдельную provenance.
- Anonymous function требует containing-scope identity.
- Parser принял syntax, но resolver не поддерживает language feature.
- Один canonical signature конфликтует внутри scope.

## Инварианты

- Formatting/reorder within scope сохраняют ID.
- Body-only change сохраняет ID и меняет revision/hash.
- Unknown target не становится exact CALLS.
- Absolute machine path не входит в ID.
- Partial adapter capability входит в graph status.
- Одинаковый relative module в разных code roots не создаёт identity collision.

## Связь с кодом и проверки

Baseline `src/adapters/symbols.rs` поддерживает Rust/Python Tree-sitter symbols, но current IDs включают start byte и relations почти пусты. Нужны new domain types, per-language contract fixtures и controlled change suite.

## Состояние реализации

Механизм отсутствует и относится к DT6.
