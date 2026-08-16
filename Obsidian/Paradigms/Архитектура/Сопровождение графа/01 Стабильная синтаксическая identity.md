---
id: "PAR-FS-006"
title: "Стабильная синтаксическая identity"
status: "принято"
implementation_stage: "будущее"
tdr_refs: ["TDR-FS-1.2"]
tdr_coverage: "прямое"
updated: "2026-08-15"
---
# Стабильная синтаксическая identity

[← Сопровождение графа](<00 Сопровождение графа.md>)

## Статус

Принята простая language-aware identity без byte offsets и без попытки автоматически сопровождать произвольные переносы между modules.

## Контекст

Форматирование, новые комментарии и перестановка methods внутри class не меняют сущность функции. ID на основе `start_byte` создаёт ложное delete/add и обесценивает semantic overlay.

## Парадигма

Language analyzer строит normalized AST и symbol table. Stable key использует logical root, relative module, containing scopes, symbol kind/name и canonical signature. Body, whitespace, comments и physical declaration order не входят в identity.

## Границы

- Resolver учитывает imports, aliases, overloads и language-specific scope rules.
- Exact static, inferred dynamic и unresolved target различаются.
- Rename/signature change допустимо считать removed+added.
- Перенос функции между modules или несвязанными classes не требует lineage matching и пересчитывается заново.
- Git heuristics и neural similarity могут помогать диагностике, но не входят в обязательный identity contract.

## Инварианты

- Reformat и move within unchanged containing scope сохраняют node ID.
- Body-only change сохраняет node ID и создаёт новую observed revision.
- Две overloads не получают один key.
- Unknown resolver result не превращается в exact edge.
- Stable key не содержит absolute machine path.

## Связи

- [TDR-FS-1.2](<../../../Docs/TDR/TDR-FS-1.2 Иерархический граф кода и identity.md>) — canonical key и language adapter.
- [Иерархический граф кода](<../Графы знаний/02 Иерархический граф кода.md>) — node hierarchy.

## Связь с реализацией

Current Rust/Python symbol IDs используют source position. Новый contract требует migration и regression fixtures, но не обязан сохранять текущие тестовые CFMap/symbol IDs.

