---
tdr_id: "TDR-FS-2.1"
title: "Каталог и admission рабочей области"
status: "принято"
implementation_stage: "текущее"
parent_tdr_id: "TDR-FS-2"
child_tdr_ids: []
updated: "2026-08-16"
---
# TDR-FS-2.1 — Каталог и admission рабочей области

## Контекст TDR

- [TDR-FS-2](<TDR-FS-2 Workspaces и terminal UX.md>) — parent package.
- [PAR-FS-011](<../../Paradigms/Архитектура/Рабочие области и интерфейс/01 Рабочая область и два контура источников.md>) — product boundary.

## Входы и результат

Входы: optional launch path, global catalog, выбранный workspace root и discovery observations. Результат: admitted workspace profile со stable workspace ID, canonical root, двумя optional source contours, stable root IDs, relative paths, exclusions и analyzer settings.

## Механизм

Global catalog содержит только workspace ID, display name, canonical path, last-opened timestamp, compatible schema version и последний health hint. Workspace configuration является authority для source roots и хранится в `.fastsearch/workspace.toml`.

Discovery ограничивается выбранным workspace root и использует typed signals:

- documentation: Markdown/TSV density, frontmatter, links, `docs`/`documentation`, Obsidian markers;
- code: repository markers, language manifests, supported extensions и analyzer availability.

Discovery создаёт предложения. Admission происходит после видимого summary/confirmation либо явного direct CLI input. Каждый contour содержит ordered roots; root identity не выводится только из display path.

## Ошибки и граничные случаи

- Workspace root отсутствует, не является directory или недоступен.
- Current directory находится сразу в нескольких overlapping registered workspaces.
- Discovery находит вложенный repository внутри другого candidate root.
- Два roots имеют одинаковое имя или одинаковые relative locators.
- Один physical root выбран для обоих contours.
- Global catalog потерян, повреждён или содержит moved workspace.
- Ноль contours подтверждены: профиль сохраняется как incomplete, query disabled.

## Инварианты

- Disk-wide automatic scan отсутствует.
- Canonical containment проверяется до записи workspace metadata.
- Root ID стабилен при неизменной workspace configuration и предотвращает locator collisions.
- Documentation и code analyzers не используют общую эвристику распознавания.
- Один contour может быть добавлен, удалён или rescanned независимо от другого.
- Повторное подключение canonical root восстанавливает catalog entry из `.fastsearch` без тяжёлого rebuild до проверки состояния.

## Связь с кодом и проверки

Typed `WorkspaceProfile`, два закрытых `SourceContour`, stable `SourceRoot`, `WorkspaceCatalog` и bounded `DiscoveryReport` реализованы в `src/application/workspace.rs`. Contract suite покрывает 0/1/2 contours, multiple roots, catalog recovery, deepest nested workspace resolution, containment и duplicate relative locators через rooted IDs. Moved workspace обрабатывается как unavailable catalog entry с повторным подключением canonical root; disk-wide recovery отсутствует.

## Состояние реализации

Workspace human runtime использует persistent catalog и `ProductionConfig::for_workspace` с optional multi-root contours. Legacy `SessionContext`/three-path construction остаётся только compatibility surface direct CLI.
