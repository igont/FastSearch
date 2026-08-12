# FastSearch — исправленный handoff DT2 → DT3

- Дата фиксации: `12.08.2026 12:08`
- DT2 product integration: `9f2ddf54873870b1cb1e332142e9d41c51c44f62`
- Исторический архив DT2: `be0b66ce0107b769e41a2a6da01de85a0ed4bca5`
- Финальный merge DT2 в `main`: `adc9ad95842047bf5b6c47cba127d7f40eeb09c9`
- Коррекция admission перед DT3: `77a05be945970a371537ea5c84e34bf11aaa8f52`
- Эффективная версия исполненного плана DT2: `PV-4`

## Причина коррекции

Первый ZIP-архив DT2 остаётся неизменяемым историческим артефактом, но его `Интеграционный отчёт.md` содержит незаполненный шаблон. Продуктовый Git-результат от этого не меняется. Для передачи следующему дереву выпускается отдельный архив `...-R1.zip` с фактическим отчётом и указанием post-merge коррекции.

Реальная проверка на целевой папке выявила три базовых расхождения между синтетическими fixtures и документальным vault:

1. frontmatter содержал допустимые однострочные YAML collections (`[a, b]`), которые parser ошибочно отвергал;
2. минимальный root-only parser `.gitignore` не поддерживал обычные glob, negation и вложенные правила;
3. неизменившийся corpus повторно создавал Tantivy projection при неизменной durable generation.

Все три расхождения закрыты причинными тестами. Ошибки источника теперь включают repo-relative locator.

## Проверенный реальный результат

Путь source root передаётся аргументом CLI. Путь `C:\Users\Igor\Downloads\Obsidian` использовался только как внешний test input и нигде не записан в product code или test fixtures.

Service root был передан отдельно как `C:\Users\Igor\Downloads\Obsidian\.cfknowledge`. Эта зона входит в явные exclusions scanner и не попадает обратно в индекс.

На проверенном snapshot:

- файлов в исходном vault: `971`, суммарно `8 805 308` байт;
- поддерживаемых источников: `918` Markdown и `30` TSV;
- успешных source snapshots в SQLite: `948`;
- канонических записей: `9 350`;
- записей metadata: `135 397`;
- первый `index rebuild`: `Current`, state/projection generation `1/1`, около `8 660 ms` в debug-запуске;
- неизменившийся `index update`: generation осталась `1/1`, около `3 719 ms`; marker projection не переписан;
- exact/FTS contracts, add/change/delete, reopen, failure→stale→rebuild recovery и mock audit проходят автоматические regression tests.

Эти времена являются единичным локальным debug evidence, а не performance SLA.

## Принятые границы после DT2

| Область | Фактическое состояние |
|---|---|
| Source | Real: параметризованный filesystem root, Markdown/TSV, `.gitignore`, exclusions |
| State | Real: SQLite authority, hashes, full-set reconciliation, reopen |
| LexicalRetrieval | Real: Tantivy exact/FTS/ranking, rebuildable projection |
| Runtime/CLI | Real: `init`, `index update/rebuild`, `search`, `get`, `status` |
| VectorRetrieval | Unavailable, принадлежит DT3 |
| CodeMaps | Unavailable, принадлежит DT3 |
| Symbols | Unavailable, принадлежит DT3 |
| Agent transport | Не завершён, принадлежит DT4 |

Сохраняемые публичные контракты находятся в `src/domain`, `src/ports` и `src/application`. Regression oracle включает source/state/lexical/runtime/CLI suites; production mock route удалён.

## Что не является доказанным

- no-op update всё ещё сканирует и сравнивает полный corpus; измерение около `3.7 s` требует release-baseline и профилирования до выбора оптимизации;
- Obsidian corpus доказывает document-format admission, но не задаёт перечень языков и parser matrix для code navigation;
- embeddings provider, vector storage/fusion, `.cfmap.md`, Tree-sitter languages и symbol identity ещё не выбраны;
- PowerShell-эргономика передачи literal quoted phrase отдельно не квалифицирована; Rust/CLI oracle quoted Russian phrase проходит;
- hard-crash, concurrency, hostile-scale, Linux packaging и production performance остаются непроверенными.

## Gate начала DT3

Перед материализацией полного дерева 3 Root обязан:

1. принять exact baseline не ниже `77a05be945970a371537ea5c84e34bf11aaa8f52` и прочитать исправленный архив R1;
2. повторно снять inventory target corpus из переданного root без hardcode;
3. зафиксировать query relevance dataset и release-mode latency/index-size baseline;
4. определить реальные code languages, `.cfmap.md` corpus и availability/privacy constraints embeddings provider;
5. открыть только bounded A1 discovery/spike horizon; vector/map/symbol implementation остаётся dormant до evidence и review.

Блокеров для начала подготовительного A1 нет. Блокеры для прямого запуска реализации vectors/maps/symbols перечислены выше и должны быть превращены в измеримые решения внутри DT3, а не угаданы заранее.
