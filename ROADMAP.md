# FastSearch — общая дорожная карта

## Назначение

FastSearch создаётся как один локальный инструмент с общим ядром, CLI и агентским интерфейсом. Основной продуктовый контур проходит через DT1-DT4. Полный граф выделен в FastGraph, а следующим отдельным результатом FastSearch становится поставщик снимков DT12.

Эта дорожная карта фиксирует общий смысл деревьев и границы между ними. Она не заменяет материализованный план текущего дерева. Прежние паспорта DT5-DT11 сохранены как заменённая история, а действующий следующий паспорт DT12 находится в [Obsidian/Docs/Roadmap](Obsidian/Docs/Roadmap/00%20Roadmap.md).

## Фактическое состояние на 16.08.2026

- Дерево 1 принято как контрактный baseline.
- Дерево 2 принято и слито в `main` ревизией `adc9ad95842047bf5b6c47cba127d7f40eeb09c9`.
- Переходная коррекция `77a05be945970a371537ea5c84e34bf11aaa8f52` закрывает admission реального Obsidian-хранилища: параметризованный root, полные `.gitignore`-правила, inline frontmatter collections, локальная исключённая зона `.cfknowledge` и no-op projection update.
- `C:\Users\Igor\Downloads\Obsidian` является проверенным представительным документальным корпусом, но не частью runtime-конфигурации и не захардкожен в коде или тестах.
- Дерево 3 принято и слито в `main` ревизией `5b25f5bf235309761f4376dc4143b246c8409c66`. Оно добавило named document/code roots, локальный E5 vector contour с честным fallback, `.cfmap.md`, Rust/Python structural symbols, deterministic fusion и единую production-композицию CLI.
- После принятого DT3 baseline в `main` слит runtime/CLI refactor; точный planning baseline DT4 — `23ed8773f9830bf6762f058255b17cbb1fe7ad46` (`main == igont/main`). Исторически E5 был cache-gated; текущий product runtime автоматически восстанавливает pinned cache и проверяет immutable runtime manifest без ручного model path. Provisioning не запускает indexing/search.
- Дерево 4 материализовано как план `FASTSEARCH-DT4-AGENT-TOOL-13-08-2026`, `PV-6`, в `.agents/DT-13-08-2026_19-10-Агентский-MCP-инструмент-FastSearch`. Только Foundation `A1→A4` имеет executable contract; B–E — `DORMANT OUTLINE` до GA/GM-PRODUCTION. Последние review потребовали развести shared `.cfknowledge` и product-owned state, planning/execution revisions и revision-scoped baseline gates; PV-6 материализует эти исправления и самостоятельно проверен Root без нового reviewer по указанию владельца. Product implementation не начата.
- Historical [DT3→DT4 handoff](evidence/dt3-dt4-handoff.md) сохраняет принятый DT3 product baseline; для strict agent admission, flat marker-owned `.cfknowledge/fastsearch-dt4-<instance_id>`, SDK lifecycle, error carriers, vector authority, accounting, limits и gates его явно supersede-ит PV-6.
- Owner decision 16.08.2026 по [workspace и terminal UX](Obsidian/Paradigms/Архитектура/Рабочие%20области%20и%20интерфейс/00%20Рабочие%20области%20и%20интерфейс.md) реализован в основном human runtime: один системный executable, persistent catalog областей, ровно два optional source contours с multiple roots, `.fastsearch` namespace и terminal-first routing. Это не переписывает DT3 acceptance; direct three-path CLI сохранён как compatibility surface.
- Retrieval model provisioning расширен до одного selectable workspace slot: E5 Small/Base/Large, Qwen3 Embedding 0.6B и Nomic Embed Text v2 MoE. Загружается и запускается только выбранная модель; readiness не инициирует index/update/search. E5 Small остаётся default и immutable-qualified baseline, остальные варианты — candidates до общего corpus benchmark. `/experiment record` материализует query/hits/latency/judgment в portable `.fastsearch/knowledge`.
- TDR-FS-2.5 реализован базовым model-evaluation slice: persistent revision-scoped vector partitions, read-only `/compare` readiness, подтверждённый `/update`, shared lexical baseline и отдельная выдача всех ready models. Обычный режим остаётся single-model. До qualification нового default остаются real-model cross-restart acceptance, disk preflight и versioned full-run evidence.
- Materialized DT4 PV-6 предшествует этому owner decision. До `G-EXECUTION-BASE` его agent-facing workspace profile и storage leaves должны быть пересмотрены против текущего [TDR-FS-2](Obsidian/Docs/TDR/TDR-FS-2%20Workspaces%20и%20terminal%20UX.md); прежний `.cfknowledge` target нельзя исполнять как актуальный owner contract.
- Перед implementation обязателен `G-EXECUTION-BASE`: после отдельного разрешения владельца текущие PV-6 `ROADMAP.md` и handoff фиксируются docs-only commit, затем exact hash/ancestry записываются как `execution_revision`. Planning snapshot не считается execution baseline. После этого `G-BASE@execution`, а затем `G-BASE@A1`…`G-BASE@A4` исполняются заново на exact revision своей стадии; исторический PASS не переиспользуется. Пока gate не закрыт, worktree A не создаётся.

## Общие правила движения

1. Одновременно материализуется и исполняется только одно дерево.
2. Будущее дерево может иметь принятый подробный функциональный паспорт, но до старта не получает executable branches/leaves и не выдаётся за implementation contract exact baseline.
3. Разработка идёт через TDD: причинный `RED` → минимальный `GREEN` → `REFACTOR` без изменения поведения → итоговый `PASS`.
4. В первом дереве внешние механизмы разрешено заменять явными mock/in-memory adapters. Оркестрация, модели, ошибки и переходы данных остаются настоящими.
5. Последующие деревья прогоняют ту же contract suite против реальных adapters и удаляют соответствующие runtime-заглушки.
6. Заглушка никогда не выдаётся за реальную возможность. Активный backend и его ограничения должны быть видны в status и тестовом evidence.
7. Детализация и полировка не расширяют текущее дерево, если не требуются для его наблюдаемого результата.

## Правило локальных спайков

Спайки добавляются постепенно и только для неизвестности, способной изменить контракт, границу компонента, хранилище либо последовательность следующих деревьев.

### Базовый минимум первого дерева

1. Преобразовать один Markdown-раздел и одну TSV-строку в единую каноническую запись.
2. Проверить на малом наборе Tantivy-поиск по русской фразе и точному техническому identifier.
3. Проверить в временной SQLite цикл `добавление → изменение → удаление` по content hash.

### Когда добавляется новый спайк

Новый спайк появляется только при зафиксированном trigger:

- выбранная библиотека не подтверждает требуемый контракт;
- неизвестность может изменить публичный тип, stable identifier или storage schema;
- реальный workload может сделать выбранный механизм неприемлемым;
- внешний протокол или parser не удаётся изолировать за согласованным port;
- evidence опровергает существенное предположение текущего плана.

Если trigger отсутствует, дополнительный спайк не создаётся. Его тема остаётся в будущем дереве без преждевременной реализации.

## Правило TODO для актуализации контрактов

Если мок, спайк или реальный adapter показывает, что ранний port больше не соответствует фактической потребности, изменение не выполняется молча. Рядом с контрактом или проверяющим тестом добавляется конкретный TODO:

```text
TODO[DT-N][область]: актуализировать <точный тип, метод, поле или поведение>.
Причина: <какое evidence показало несоответствие>.
Требуемое состояние: <что должно наблюдаться после изменения>.
Gate удаления TODO: <какой тест или проверка подтверждает результат>.
```

TODO обязан называть, что именно требуется актуализировать. Формулировки `доделать`, `улучшить` и `разобраться` без объекта и gate не допускаются.

TODO блокирует закрытие текущего дерева, если относится к его принимаемому результату. TODO может быть передан следующему дереву только когда в дорожной карте явно указано, почему текущий результат без него остаётся целостным.

### Плановая замена ранних ports

| Ранняя граница | Временное состояние | Что именно актуализируется | Целевое дерево |
|---|---|---|---:|
| `SourcePort` и `DocumentParser` | Fixtures и mock records | Реальный обход файлов, Markdown sections, frontmatter и TSV rows | 2 |
| `StateStore`, `LexicalIndex`, `Ranker` | In-memory state и предопределённая выдача | Hash lifecycle, SQLite, exact lookup, FTS и режимы ranking | 2 |
| `VectorIndex`, `CodeMapService`, `SymbolExtractor` | Отключённые либо mock capabilities | Реальные vectors, карты, symbols и единая выдача | 3 |
| `AgentSurface` и `Capability::AgentSurface` | Общий application port без protocol adapter; capability не объявлена доступной | Локальный MCP adapter поверх той же production-композиции, typed DTO и наблюдаемый transport status | 4 |

Незапланированное изменение этой таблицы требует нового evidence и проверки влияния на следующие деревья.

## Дерево 1 — Контрактный каркас и проверка гипотез

### Образ результата

На столе собран испытательный стенд будущей системы. Через него уже проходит весь основной поток, но внешние механизмы представлены контролируемыми макетами. Стенд показывает, что части соединяются правильно и реальные детали позднее можно заменять по одной.

### Вход

- согласованный общий смысл FastSearch;
- исходные описания Knowledge Search и Code Navigation;
- небольшой синтетический набор fixtures;
- чистый Git baseline;
- Rust toolchain, доступность которого подтверждается перед материализацией дерева.

### Что делает дерево

- создаёт Cargo-каркас с единым core;
- фиксирует каноническую поисковую запись и основные ports;
- создаёт contract tests и сквозные golden-сценарии;
- подключает явные mock/in-memory adapters;
- проводит только базовый минимум локальных спайков;
- доказывает mock-поток `init → index → search/get/related/status`.

### Какие проблемы решает

- устраняет неопределённость общей формы программы;
- проверяет связность компонентов до тяжёлой реализации;
- создаёт test oracle для последующей замены заглушек;
- рано выявляет несовместимость модели записи, поиска и состояния.

### Какие проблемы не решает

- не обещает качественный поиск по реальному корпусу;
- не реализует полный parser, постоянный индекс, vectors или symbols;
- не доказывает производительность;
- не занимается полировкой интерфейсов и эксплуатационных сценариев.

### Состояние на завершении

- mock-поток проходит сквозные тесты;
- каждый mock привязан к явному port и будущему дереву замены;
- базовые спайки имеют воспроизводимое evidence;
- все выявленные изменения ports описаны точными TODO;
- границы второго дерева можно заново исследовать на принятом baseline.

## Дерево 2 — Реальный поиск по документам

### Образ результата

Макет источников и поискового двигателя заменён рабочим механизмом. Инструмент получает реальные документы, строит локальное производное состояние и возвращает объяснимую выдачу через CLI.

### Вход

- принятый каркас и contract suite первого дерева;
- подтверждённые решения базовых спайков;
- каноническая запись и стабильные ports либо точные TODO на их актуализацию;
- тестовый корпус и набор regression queries.

### Что делает дерево

- реализует обход файлов и exclusions;
- разбирает Markdown, frontmatter, sections и TSV rows;
- реализует hashes, SQLite и lifecycle `rebuild/update/delete`;
- подключает exact lookup и Tantivy FTS;
- реализует ranking и режимы `BALANCED`, `CURRENT`, `DESIGN`;
- заменяет относящиеся к этому потоку mocks реальными adapters.

### Какие проблемы решает

- даёт первый реально полезный документальный поиск;
- обеспечивает повторяемое обновление локального индекса;
- различает точный identifier, полнотекстовую релевантность и статус знания;
- проверяет общую модель на реальном корпусе.

### Какие проблемы не решает

- не отвечает за семантический vector search;
- не строит code maps и symbol index;
- не предоставляет законченный агентский transport;
- не выполняет позднюю оптимизацию и выпуск.

### Состояние на завершении

- CLI индексирует тестовый корпус и возвращает стабильные результаты;
- add/change/delete подтверждены причинными тестами;
- exact и FTS проходят regression dataset;
- mocks документального потока удалены из runtime;
- оставшиеся capabilities явно показываются как mock либо unavailable.

## Дерево 3 — Семантическая и кодовая навигация

### Образ результата

Поиск начинает понимать не только точные слова, но и смысл запроса, а также ведёт от архитектурного описания к конкретной области и symbol. Разные виды знания сходятся в одной объяснимой выдаче.

### Вход

- принятый реальный документальный поиск и исправленный DT2→DT3 handoff;
- стабильные record, retrieval и state contracts;
- параметризованный реальный корпус, inventory языков/source kinds и воспроизводимый baseline качества и времени поиска;
- подтверждённое разделение source root и исключённой service-зоны `.cfknowledge` внутри него либо в другом переданном пути;
- локальные discovery/spike-листья только для реально возникших неизвестностей.

### Что делает дерево

- реализует embeddings adapter, локальные vectors и штатный fallback;
- реализует lifecycle `.cfmap.md` с AUTO/CURATED и stale-состоянием;
- извлекает code symbols и locators через Tree-sitter;
- объединяет exact, FTS и vector candidates;
- расширяет `related` и regression dataset для смешанных запросов.

### Какие проблемы решает

- находит сведения при смысловом перефразировании;
- сокращает путь от вопроса к нужной области кода;
- связывает документы, карты и symbols единым retrieval contract;
- сохраняет работоспособность exact/FTS без vector provider.

### Какие проблемы не решает

- не завершает внешний агентский protocol;
- не обещает compiler-perfect references для всех языков;
- не вводит HNSW без подтверждённой необходимости;
- не выполняет финальную эксплуатационную полировку.

### Состояние на завершении

- все основные source kinds участвуют в общей выдаче;
- semantic fallback и отсутствие provider проверены;
- карты сохраняют curated content при update;
- symbols имеют стабильный source locator;
- относящиеся к навигации runtime-mocks удалены.

### Фактически принято

- `AgentSurface` является общей application-границей `search/get/related/status/index_status`, а не transport-реализацией;
- `ProductionRuntime` соединяет SQLite authority, Tantivy, optional local E5, maps, symbols и fusion;
- CLI открывает production runtime и не содержит отдельной поисковой логики;
- E5 model artifacts остаются внешним локальным cache, проверяемым по immutable manifest, и не входят в исполняемый файл;
- test-only `BackendKind::Mock` и `ReferenceFixture` сохранены как oracle, но production mock route отсутствует;
- compiler-resolved references, дополнительные языки, Linux qualification и MCP transport не заявлены результатом DT3.

Выделение FastGraph не переписывает acceptance DT3 задним числом. `.cfmap.md` и базовые symbols остаются честно принятым результатом DT3 и поддерживаемой узкой навигацией FastSearch, но не определяют целевую модель FastGraph.

## Дерево 4 — Агентский доступ и готовый инструмент

### Образ результата

Принятый retrieval core получает узкий локальный MCP-вход. CLI и MCP используют одну production-композицию и возвращают семантически одинаковые результаты, ошибки, provenance и freshness, хотя их текстовое представление различается. Инструмент запускается и диагностируется без знания внутренних каталогов индекса.

### Вход

- exact local `main` planning snapshot `23ed8773f9830bf6762f058255b17cbb1fe7ad46`, historical DT3 product baseline `5b25f5bf235309761f4376dc4143b246c8409c66` и прочитанный archive/handoff/PV-6; execution revision назначается только `G-EXECUTION-BASE`;
- принятые `AgentSurface`, `ProductionConfig` и `ProductionRuntime` без transport-specific типов внутри domain;
- завершённые document, lexical, optional vector, map и symbol adapters;
- regression dataset, DT3 release evidence и повторная проверка доступности локального E5 cache;
- инвентаризация test-only mocks, protocol gaps, устаревших комментариев и реально открытых TODO;
- принятые до implementation решения по dual-era envelope, DTO/redaction, strict `instance_id` + opaque admitted root, map-source related, exact outcome/legacy projection, agent-scoped vector health и single-owner concurrency; B/C/D1/D2 получают configurable bounds, а release numbers замораживаются только D3.

### Что делает дерево

- добавляет в тот же executable локальный MCP server; базовый кандидат transport — `stdio`, а любой listener/remote contour требует отдельного evidence и replan;
- преобразует MCP input/output через отдельные typed protocol DTO, не сериализуя domain-модель как случайный публичный wire contract;
- предоставляет agent tools для `search`, `get`, `related` и `status` поверх того же `AgentSurface`/`ProductionRuntime`;
- определяет indexing ownership явно: `update/rebuild` остаются operator CLI либо становятся отдельными maintenance tools, но никогда не выполняются скрыто внутри каждого search request;
- проверяет semantic parity CLI/MCP на общей contract suite: records, ordering, channels, provenance, freshness и structured errors;
- вводит измеренные ограничения query length, result count, payload bytes, execution time и одновременно обслуживаемой работы; truncation и cancellation наблюдаемы;
- historical PV-6 фиксирует agent profile для document/code roots, logical instance id и optional model root и помещает state в `.cfknowledge`; перед execution этот leaf должен быть заменён workspace profile и `.fastsearch/local` storage по TDR-FS-2 без раскрытия physical paths в wire;
- проверяет lifecycle долгоживущего процесса: startup, повторные requests, stale/degraded recovery, provider absence/failure и controlled shutdown;
- готовит воспроизводимую Windows-first сборку, checksum, smoke из чистого каталога и документацию запуска MCP-клиентом;
- сохраняет optional model cache внешним immutable artifact: «один бинарник» означает один FastSearch executable, а не встраивание весов E5;
- подтверждает отсутствие production mock route и объявляет `Capability::AgentSurface` доступной только при реально запущенном protocol adapter.

### Какие проблемы решает

- предоставляет стабильный локальный агентский интерфейс без дублирования retrieval logic;
- устраняет дорогой shell/process и текстовый parsing boundary для каждого agent request;
- делает protocol, indexing, provider и storage failures различимыми и наблюдаемыми;
- ограничивает контекст и ресурсы на protocol boundary, а не полагается на дисциплину клиента;
- подтверждает пригодность Windows-сборки для регулярного локального использования;
- закрывает воспроизводимые release и эксплуатационные gates в заявленном platform scope.

### Какие проблемы не решает

- не превращает FastSearch в RAG-chat;
- не добавляет HTTP service, multi-user daemon, cloud inference, auth subsystem или отдельную внешнюю поисковую платформу без нового evidence;
- не расширяет scope новыми пользовательскими функциями;
- не переоткрывает принятые контракты без регрессии или нового риска.
- не обещает compiler-perfect references, semantic refactoring, dirty-buffer overlay, новые языки или Linux qualification;
- не встраивает model weights в executable и не превращает optional E5 в обязательное условие lexical/code navigation.

### Состояние на завершении

- локальный MCP client поднимает server, выполняет `search/get/related/status` и корректно завершает процесс;
- CLI и MCP проходят общую semantic contract suite, включая ordering, provenance, freshness и typed failures;
- startup configuration однозначна, machine paths не протекают в protocol payload, скрытых index mutations нет;
- обязательные runtime-capabilities используют реальные adapters, а `AgentSurface` честно виден как Real только в MCP composition;
- отсутствие optional provider даёт штатную деградацию без потери exact/FTS/maps/symbols;
- query/result/payload/time/concurrency limits и context-economy measurements имеют причинное evidence;
- выпускается проверенный Windows executable с checksum, fresh-directory smoke и инструкцией подключения локального MCP-клиента.

### Gate materialization

Перед материализацией дерева 4 требовался bounded discovery. Его решения теперь канонизированы PV-6, а фактические evidence производятся Foundation/D3 gates:

1. выбрать и зафиксировать protocol/SDK version и проверить минимальный Rust `stdio` handshake на текущем toolchain;
2. определить wire DTO и точное отображение domain errors/status без потери provenance;
3. согласовать startup configuration и ownership `index update/rebuild`;
4. выбрать последовательную либо явно синхронизированную модель долгоживущего runtime;
5. измерить core/actor в A4 и modern+legacy production stdio в D3; только D3 назначает release numeric limits и закрывает `G-LIMITS`, это не блокирует B/C/D1/D2;
6. A4 вызывает текущий automatic provisioning contract TDR-FS-2.4 и фиксирует полученную immutable E5 revision в evidence; ручная доставка cache больше не является prerequisite. Недоступность сети/cache оставляет vector acceptance `NOT_READY`, но не делает E5 обязательным условием lexical/code navigation.

## Передача полного графа и следующий этап

Прежние DT5-DT11 переданы [FastGraph](Obsidian/Docs/FastGraph.md) и сохранены только как заменённая история. Их идентификаторы не переиспользуются. FastSearch больше не планирует полный документальный и кодовый граф, графовые ревизии, переносимый семантический слой и интеграцию GraphProvider.

| Этап | Производящая зависимость | Наблюдаемый результат | Состояние | Подробный паспорт |
|---|---|---|---|---|
| DT12. Поставщик снимков для FastGraph | Принятая среда рабочих областей и граница FastGraph FG0 | Независимый клиент получает точный ограниченный снимок допущенных исходных единиц | предстоит | [DT12](Obsidian/Docs/Roadmap/08%20DT12%20Поставщик%20снимков%20для%20FastGraph.md) |

DT12 сохраняет FastSearch владельцем допуска путей и чтения источников, но не владельцем графа. FastGraph FG1 является первым реальным потребителем результата DT12. Dtree продолжает получать поисковые кандидаты от FastSearch, а графовое расширение подключает к FastGraph на собственном этапе R6.

## Переходы между деревьями

Каждый переход фиксирует:

```text
принятый результат → exact baseline → сохранённые contracts/tests →
активные TODO и оставшиеся mocks → evidence gaps → gate следующего дерева
```

Следующее дерево не наследует предположения как факты. Перед его материализацией проверяются фактический baseline, актуальность TODO, оставшиеся mocks и результаты предыдущих спайков. Если изменились результат, scope, ownership, dependency, публичный контракт либо acceptance gate, контур следующего дерева пересматривается до запуска.

Фактический переход DT2→DT3 зафиксирован в `evidence/dt2-dt3-handoff.md`. Фактический переход DT3→DT4 зафиксирован в `evidence/dt3-dt4-handoff.md`: он отделяет принятый retrieval/runtime baseline от ещё не выбранного protocol и release contour.
