---
tdr_id: "TDR-FS-2.2"
title: "Namespace .fastsearch и migration"
status: "принято"
implementation_stage: "запланированное"
parent_tdr_id: "TDR-FS-2"
child_tdr_ids: []
updated: "2026-08-16"
---
# TDR-FS-2.2 — Namespace `.fastsearch` и migration

## Контекст TDR

- [TDR-FS-2](<TDR-FS-2 Workspaces и terminal UX.md>) — parent package.
- [PAR-FS-009](<../../Paradigms/Архитектура/Сопровождение графа/04 Переносимый namespace .fastsearch.md>) — ownership и portability.
- [Передача FastGraph](<../FastGraph.md>) - переносимое графовое знание и локальный граф больше не принадлежат `.fastsearch`.

## Входы и результат

Входы: admitted workspace, `.fastsearch` schema version, optional legacy `.cfknowledge`/external service state и optional legacy `.search`. Результат: pinned workspace store с разделёнными portable и local branches, migration report и однозначной ownership boundary.

## Layout

```text
.fastsearch/
  workspace.toml
  knowledge/
    experiments/
  local/
    index/
      documents/
      code/
      cross/
        lexical/
      vector/
        <model-slug>/
          <model-revision>/
            manifest.toml
            vectors.bin
            records.sqlite
    state.sqlite
    cache/
    runtime/
```

- `workspace.toml` и `knowledge/experiments` являются переносимыми данными FastSearch.
- `.fastsearch/local/` целиком исключён из Git и воспроизводим из sources и поисковой конфигурации.
- Inner directories не получают ведущую точку: верхний `.fastsearch` уже образует единый hidden product namespace.
- Авторитетное состояние источников и полнотекстовая проекция являются общими. Только векторная проекция разделяется по `model-slug/model-revision`; дублирование всего `.fastsearch/local` для каждой модели запрещено.
- Model partition manifest связывает repository/revision, runtime contract, dimension, corpus fingerprint, canonical generation и record count. Несовпадение любого causally significant поля делает partition stale.

## Механизм

Storage manager проверяет containment рабочей области, schema marker, symlink/reparse traversal, ownership marker и lock до изменения. Переносимые записи используют воспроизводимую сериализацию и атомарную замену. Локальные индексы и состояние используют transaction/commit markers и могут быть отброшены после прерванного запуска.

Migration выполняется явно и идемпотентно:

1. Обнаружить legacy external service root, `.cfknowledge` instance или root `.search`.
2. Проверить ownership и совместимую schema/provenance.
3. Сформировать preview с источниками, целями и конфликтами.
4. Импортировать совместимую конфигурацию и поисковые эксперименты без графовых данных FastGraph.
5. Перестроить disposable local state в `.fastsearch/local`.
6. Оставить legacy data нетронутыми до подтверждённого parity и отдельной cleanup operation.

## Ошибки и граничные случаи

- `.fastsearch` является file, symlink/reparse escape или принадлежит неизвестной schema.
- Git ignore ошибочно исключает portable branch.
- Переносимая запись конфликтует с параллельным изменением конфигурации или эксперимента.
- Legacy state найдено в нескольких locations.
- Local deletion происходит во время active writer.
- Workspace перемещён, а absolute locator попал в portable configuration.

## Инварианты

- FastSearch не использует CF-prefixed target directory.
- Удаление `.fastsearch/local` не удаляет конфигурацию рабочей области или поисковые эксперименты.
- Unknown schema не импортируется частично.
- Generated index, embeddings, candidates и runtime locks не требуют Git versioning.
- Удаление partition одной модели не изменяет shared state, lexical index или partitions других моделей.
- Portable paths внутри workspace сохраняются относительно canonical root.
- Migration не удаляет legacy data автоматически.

## Связь с кодом и проверки

Нужны проверки структуры хранения, ownership marker, атомарной переносимой записи, выборочного `.gitignore`, новой копии репозитория, перестроения после удаления local, Windows reparse defense, параллельной блокировки и migration fixtures для `.cfknowledge`, external service root и `.search`.

## Состояние реализации

Текущий workspace store создаёт `.fastsearch`, записывает `workspace.toml` атомарной заменой, исключает только `/local/`, проверяет containment и Windows reparse traversal и размещает SQLite/Tantivy state в `.fastsearch/local`. Удаляемое локальное состояние пересобирается из настроенных источников. Созданные ранее каталоги `knowledge/curated` и `local/knowledge` являются переходным следом прежнего графового замысла и не получают новых данных; их удаление или миграция требуют отдельного подтверждения отсутствия пользовательского содержимого. Legacy `.cfknowledge` и `.search` остаются нетронутыми.
