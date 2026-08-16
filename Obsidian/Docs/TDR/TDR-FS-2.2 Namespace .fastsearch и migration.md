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
- [TDR-FS-1.4](<TDR-FS-1.4 Semantic overlay и .fastsearch.md>) — curated semantic records.

## Входы и результат

Входы: admitted workspace, `.fastsearch` schema version, optional legacy `.cfknowledge`/external service state и optional legacy `.search`. Результат: pinned workspace store с разделёнными portable и local branches, migration report и однозначной ownership boundary.

## Layout

```text
.fastsearch/
  workspace.toml
  knowledge/
    curated/
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
    knowledge/
      graph/
      candidates/
      revisions/
    state.sqlite
    cache/
    runtime/
```

- `workspace.toml` и `knowledge/curated` являются portable data.
- `.fastsearch/local/` целиком ignored и воспроизводим из sources и curated knowledge.
- Inner directories не получают ведущую точку: верхний `.fastsearch` уже образует единый hidden product namespace.
- Canonical source state, document/code graph и lexical projection являются общими. Только vector projection разделяется по `model-slug/model-revision`; дублирование всего `.fastsearch/local` для каждой модели запрещено.
- Model partition manifest связывает repository/revision, runtime contract, dimension, corpus fingerprint, canonical generation и record count. Несовпадение любого causally significant поля делает partition stale.

## Механизм

Storage manager проверяет canonical workspace containment, schema marker, symlink/reparse traversal, ownership marker и lock до создания или mutation. Portable writes используют deterministic serialization и atomic replace. Local index/state writes используют transaction/commit markers и могут быть отброшены после interrupted run.

Migration выполняется явно и идемпотентно:

1. Обнаружить legacy external service root, `.cfknowledge` instance или root `.search`.
2. Проверить ownership и совместимую schema/provenance.
3. Сформировать preview с источниками, целями и конфликтами.
4. Импортировать curated data без автоматического принятия generated candidates.
5. Перестроить disposable local state в `.fastsearch/local`.
6. Оставить legacy data нетронутыми до подтверждённого parity и отдельной cleanup operation.

## Ошибки и граничные случаи

- `.fastsearch` является file, symlink/reparse escape или принадлежит неизвестной schema.
- Git ignore ошибочно исключает portable branch.
- Curated export конфликтует с concurrent review commit.
- Legacy state найдено в нескольких locations.
- Local deletion происходит во время active writer.
- Workspace перемещён, а absolute locator попал в portable configuration.

## Инварианты

- FastSearch не использует CF-prefixed target directory.
- Удаление `.fastsearch/local` не удаляет workspace configuration или accepted knowledge.
- Unknown schema не импортируется частично.
- Generated index, embeddings, candidates и runtime locks не требуют Git versioning.
- Удаление partition одной модели не изменяет shared state, lexical index или partitions других моделей.
- Portable paths внутри workspace сохраняются относительно canonical root.
- Migration не удаляет legacy data автоматически.

## Связь с кодом и проверки

Нужны storage-layout contract, ownership marker, atomic portable writer, selective `.gitignore` fixture, fresh-clone round-trip, local-delete rebuild, Windows reparse defense, concurrent lock tests и migration fixtures для `.cfknowledge`, external service root и `.search`.

## Состояние реализации

Текущий workspace store создаёт нормативный `.fastsearch` layout, записывает deterministic `workspace.toml` atomic replace, исключает только `/local/`, проверяет containment и Windows reparse traversal и размещает SQLite/Tantivy state в `.fastsearch/local`. Удаляемое local state пересобирается из configured sources. Legacy `.cfknowledge` и `.search` обнаруживаются и остаются нетронутыми. Полный migration report, schema-validated curated import, external-service discovery и parity-gated cleanup остаются запланированным механизмом; поэтому стадия всего TDR-FS-2.2 не повышена до `текущее`.
