---
id: "PAR-FS-009"
title: "Переносимый namespace .fastsearch"
status: "принято"
implementation_stage: "запланированное"
tdr_refs: ["TDR-FS-2.2"]
tdr_coverage: "прямое"
updated: "2026-08-16"
---
# Переносимый namespace `.fastsearch`

[← Сопровождение графа](<00 Сопровождение графа.md>)

## Статус

Принята единая граница хранения FastSearch. Она содержит переносимую конфигурацию рабочей области, поисковые эксперименты и локальные воспроизводимые проекции. Принятые графовые знания переданы FastGraph.

## Контекст

FastSearch не связан с CadFrame и не должен использовать CF-prefixed target storage. Несколько скрытых каталогов верхнего уровня усложняют очистку и диагностику. Поисковые индексы, векторные представления и временные результаты можно пересчитать, а конфигурация рабочей области должна переживать удаление локального состояния.

## Парадигма

Рабочая область содержит одну скрытую папку `.fastsearch`. В её переносимой части хранятся конфигурация workspace и результаты поисковых экспериментов. В `.fastsearch/local` находятся SQLite, поисковые индексы, векторные представления, история сканирования и runtime locks; эта ветвь целиком исключается из Git и воспроизводится.

Концептуальная граница:

```text
.fastsearch/
  workspace.toml
  knowledge/
    experiments/
  local/
    index/
    state.sqlite
    cache/
    runtime/
```

Inner directories не получают leading dot. Единственный hidden product namespace — `.fastsearch`.

## Ownership

- `workspace.toml` описывает portable workspace identity, два source contours, relative roots и explicit exclusions.
- `knowledge/experiments` хранит переносимые результаты явных сравнительных запусков поиска и не является графовым принятым слоем.
- `local` принадлежит экземпляру FastSearch, не является интеграционным интерфейсом и может быть удалён для полного перестроения.
- Source code и authored documentation остаются внешними canonical sources.
- Current `.cfknowledge`, external service root и root `.search` являются legacy inputs migration, а не target layout.

## Инварианты

- Git обязательно игнорирует `.fastsearch/local/`, но не весь `.fastsearch`.
- Новая копия репозитория восстанавливает конфигурацию рабочей области, затем пересчитывает локальное состояние.
- Удаление локального cache не теряет переносимую конфигурацию и сохранённые эксперименты.
- Deterministic export даёт reviewable Git diff.
- Артефакты моделей, векторные представления и временные кандидаты не коммитятся ради воспроизводимости запроса.
- Unknown schema version не импортируется частично.
- Migration не удаляет legacy state до подтверждённого parity.

## Связи

- [TDR-FS-2.2](<../../../Docs/TDR/TDR-FS-2.2 Namespace .fastsearch и migration.md>) — physical layout, Git policy и migration.
- [Рабочая область и два контура источников](<../Рабочие области и интерфейс/01 Рабочая область и два контура источников.md>) — workspace ownership.
- [Передача FastGraph](<../../../Docs/FastGraph.md>) - отдельный namespace переносимых графовых знаний.

## Связь с реализацией

Workspace runtime уже использует `.fastsearch/local` для SQLite/Tantivy state, а `workspace.toml`, `knowledge/experiments` и selective `.gitignore` материализуют переносимую и локальную границы. Созданный ранее `knowledge/curated` является переходным каталогом старого замысла и не получает новые графовые записи. Его удаление или миграция требуют отдельного подтверждения отсутствия пользовательских данных.
