---
id: "PAR-FS-009"
title: "Переносимый namespace .fastsearch"
status: "принято"
implementation_stage: "запланированное"
tdr_refs: ["TDR-FS-1.4", "TDR-FS-2.2"]
tdr_coverage: "прямое"
updated: "2026-08-16"
---
# Переносимый namespace `.fastsearch`

[← Сопровождение графа](<00 Сопровождение графа.md>)

## Статус

Принята единая product-owned storage boundary. Точные portable record filenames и sharding выбираются implementation evidence, но верхнеуровневый namespace, Git boundary и разделение accepted/local являются нормативными.

## Контекст

FastSearch не связан с CadFrame и не должен использовать CF-prefixed target storage. Несколько top-level hidden directories усложняют mental model, cleanup и диагностику. При этом structural graph, embeddings и candidates можно пересчитать, а проверенные descriptions и document↔code links должны переживать rebuild и fresh clone.

## Парадигма

Workspace содержит одну скрытую папку `.fastsearch`. В её portable части хранятся workspace configuration и accepted semantic knowledge. В `.fastsearch/local` находятся SQLite, indexes, structural graph, embeddings, scan history, runtime locks и unreviewed candidates; эта ветвь целиком исключается из Git и воспроизводится.

Концептуальная граница:

```text
.fastsearch/
  workspace.toml
  knowledge/
    curated/
  local/
    index/
    knowledge/
    state.sqlite
    cache/
    runtime/
```

Inner directories не получают leading dot. Единственный hidden product namespace — `.fastsearch`.

## Ownership

- `workspace.toml` описывает portable workspace identity, два source contours, relative roots и explicit exclusions.
- `knowledge/curated` изменяется только validated import/export или curator review commit.
- `local` принадлежит FastSearch instance, не является integration API и может быть удалён для полного rebuild.
- Source code и authored documentation остаются внешними canonical sources.
- Current `.cfknowledge`, external service root и root `.search` являются legacy inputs migration, а не target layout.

## Инварианты

- Git обязательно игнорирует `.fastsearch/local/`, но не весь `.fastsearch`.
- Fresh clone восстанавливает workspace configuration и accepted knowledge, затем пересчитывает local state.
- Удаление local cache не теряет reviewed knowledge.
- Deterministic export даёт reviewable Git diff.
- Model artifacts, embeddings и generated candidates не коммитятся ради воспроизводимости query.
- Unknown schema version не импортируется частично.
- Migration не удаляет legacy state до подтверждённого parity.

## Связи

- [TDR-FS-1.4](<../../../Docs/TDR/TDR-FS-1.4 Semantic overlay и .fastsearch.md>) — curated semantic schema и review commit.
- [TDR-FS-2.2](<../../../Docs/TDR/TDR-FS-2.2 Namespace .fastsearch и migration.md>) — physical layout, Git policy и migration.
- [Рабочая область и два контура источников](<../Рабочие области и интерфейс/01 Рабочая область и два контура источников.md>) — workspace ownership.

## Связь с реализацией

Workspace runtime уже использует `.fastsearch/local` для SQLite/Tantivy state, а `workspace.toml`, `knowledge/curated` и selective `.gitignore` материализуют portable/local boundary. Tests подтверждают layout, round-trip, selective ignore и rebuildable workspace runtime. Стадия остаётся `запланированное` для полной парадигмы, поскольку curated semantic schema, review commit, fresh-clone accepted-knowledge restore и legacy migration report относятся к последующим graph stages.
