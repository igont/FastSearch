# BC-FS-SEARCH-SOURCE-ANCHOR. Поисковая привязка к исходному материалу

## Состояние и версия

Контракт имеет версию `1` и состояние `BLOCKED`. Его смысл и обязательные поля доступны для согласования, но FastSearch пока не производит этот выход, а FastGraph ещё не объявил совместимый профиль разрешения привязки в узел графа.

## Производитель и ответственность

Единственный производитель `SearchSourceAnchor v1` - будущая граница `application::public_search`, вызываемая `ThinSearchCoordinator` при формировании каждого итогового результата. Она обязана в той же атомарной операции получить происхождение по `record_id + state_generation`, связать каноническую запись с точным логическим корнем и SHA-256 исходных байтов до любой нормализации и вложить готовую привязку в `PublicSearchResult v2`. Последующий выбор результата не запускает новый lookup и не восстанавливает identity по пути или rank. MCP, CLI, клиент композиции и FastGraph не достраивают отсутствующие поля и не становятся производителями привязки.

## Среда исполнения и платформенные порты

Форма привязки платформенно нейтральна. Текущая реализация планируется только для локального запуска на Windows. Для неё понадобятся существующие возможности допуска корня и нормализации относительного пути, а также ещё не реализованный атомарный lookup происхождения по `record_id + state_generation`. Он возвращает `contour`, `root_id`, относительный локатор и SHA-256 исходных байтов до снятия BOM или нормализации CRLF/CR. Linux и macOS остаются точками расширения `NOT_IMPLEMENTED`; абсолютный путь Windows в формат не входит.

## Потребители и назначение передачи

Единственный прямой потребитель результата этой внутренней границы - `application::thin_search`, принимающий `FinalizedAnchorAssembly` вместе с отдельным `AnchorPublicationPermit` либо один pre-lease abort/failure/termination/replay. Независимый клиент DT13-A получает привязку только внутри committed `PublicSearchResponse v2`; отдельного `SearchSourceAnchorOutcome -> serializer` пути нет. Будущий композиционный клиент извлекает outcome из того же внешнего ответа и передаёт привязку в совместимый профиль FastGraph. FastGraph не является потребителем до публикации собственной текстовой и машинной пары разрешения привязки; FastSearch не вызывает его напрямую и не получает зависимость сборки от FastGraph.

## Входные данные и авторитетные источники

Вход отдельной позиции состоит из итоговой `CanonicalRecord`, захваченного поисковым вызовом `state_generation`, повторно открытого `WorkspaceProfile` и обязательного результата атомарного lookup происхождения по `record_id + state_generation`. `CanonicalRecord` является источником истины только для `record_id`, относительного локатора, полного payload селектора и хэша канонической записи: типизированного `root_id` в нём сейчас нет. `WorkspaceProfile` подтверждает `workspace_id` и множество допущенных корней, но при нескольких корнях сам не выбирает корень записи.

Сборка ответа дополнительно получает от `application::thin_search` один `RankedCanonicalResultBundle v1` с `response_assembly_id`, захваченным поколением и полным упорядоченным набором ожидаемых `position_id + record_id`, а от Windows process identity provider - авторитетный `owner_process_identity = PID + process_start_time`. `application::public_search` всегда инициализирует `AnchorAssemblyState v1` из bundle, включая корректное пустое состояние для нулевой выдачи. Каждый provisional READY/BLOCKED payload несёт внутренние `response_assembly_id + position_id + record_id + captured state_generation`; эти поля изолируют параллельные сборки одного поколения и удаляются только после корреляции.

После завершения provisional state `application::public_search` создаёт `GenerationPublicationLeaseRequest v1` с точной `workspace identity + response_assembly_id + captured generation + owner process identity`. Владелец [generation publication lease](BC-FS-GENERATION-PUBLICATION-LEASE.md) возвращает ровно одну из трёх взаимоисключающих workspace-scoped ветвей: domain outcome `ACTIVE_ACQUIRED|REPLAYED_TERMINAL|REPLAYED_PROCESS_DIED|CHANGED|FAILED` после durable confirmation; generic `NOT_COMMITTED_RETRY|COMMIT_READBACK_REQUIRED|COMMIT_RECOVERY_BLOCKED`, если commit не доказан; либо typed `GenerationLeaseStateAccessBlocked`, если authoritative ledger потерян, повреждён или недоступен. Только `ACTIVE_ACQUIRED` несёт действующий `lease_id`; `REPLAYED_TERMINAL` дословно сохраняет `lease_id + committed_terminal_identity + terminal_journal_ref + terminal_record_digest + released_revision` из авторитетного результата coordinator. Две отказные ветви не доказывают наличие или отсутствие lease и не разрешают permit, pre-lease terminal либо новый acquire identity.

До выдачи permit `application::public_search` принимает correlated `ResponseAssemblyTerminationRequest v1`. Это входной request от thin_search, а не разрешённый результат. После single-CAS только ветвь без lease производит отдельный `PrePermitAssemblyTermination v1` обратно в thin_search; совпадение этих двух сообщений по одному ID запрещено. Общий lifecycle различает `ASSEMBLING`, `LEASE_PENDING`, `FINALIZED`, `TERMINAL` и `RECOVERY_BLOCKED`. Domain outcomes разрешаются прежним single-CAS правилом; generic commit status сохраняет ту же acquire operation: первые два recovery-кода оставляют `LEASE_PENDING`, а `COMMIT_RECOVERY_BLOCKED` переводит только операцию в `RECOVERY_BLOCKED`. Typed ledger-access block также переводит exact assembly в `RECOVERY_BLOCKED`, сохраняя operation, lookup evidence и optional retained proposal identity byte-for-byte. Обе blocked-ветви передаются thin_search, supervisor, владельцу операции и diagnostics; они не создают domain outcome, permit, public terminal, release или безопасный no-lease terminal. Никакая ветвь не предполагает, что ещё не подтверждённый lease отсутствует.

Успешный `SourceAnchorProvenanceLookupOutcome v1` создаётся будущим владельцем source/state и содержит точные `contour`, `root_id`, относительный локатор, `raw_source_sha256` и `state_generation`. Производитель привязки сверяет локатор lookup с `CanonicalRecord`, а корень - с повторно открытым профилем. Неуспешный lookup содержит типизированную причину без угаданного корня или хэша.

Lookup привязки является дополнением результата для DT13, а не предусловием базового поиска DT4. Невозможность создать привязку не отменяет уже корректный поисковый результат и не превращает весь `PublicSearchResponse` в частичный успех: она блокирует только отдельное намерение продолжить выбранный результат по графу.

Текущая модель хранит `RootedSourceLocator` только на отдельных участках и `CanonicalRecord::content_hash`, но сама запись не хранит типизированный корень. Текущий `SourceSnapshot::file_hash` также не подходит для `raw_source_sha256`: входной адаптер сначала снимает BOM и нормализует CRLF/CR в LF, затем создаёт доменно разделённый `sha256:v1:<digest>`. Хэш канонической записи и нормализованный `FileHash` не подменяют SHA-256 исходных байтов. Эти различия являются блокерами реализации.

## Выходные данные и формат

`SearchSourceAnchor v1` является закрытым JSON-объектом. Общий lifecycle выбирает одну взаимоисключающую ветвь: finalized assembly + permit; terminal/process-death replay; pre-lease abort/failure/termination; post-permit termination; либо `AnchorLeaseCommitRecoveryState` для generic commit status или typed ledger-access block. Последняя ветвь никогда не становится внешним READY/BLOCKED outcome сама по себе и не передаёт finalized assembly или permit. При pending `CANCELLED|TIMEOUT` обычные recovery transforms отключены: retry/readback сохраняет pending event, а terminal commit/access block поглощает его только через pending-aware переход. В [Public Search v2](BC-FS-PUBLIC-SEARCH-V2.md) `READY` содержит одну привязку без ошибки, а `BLOCKED` - одну ошибку без привязки; both/neither запрещены. Сам объект привязки содержит:

| Поле | Обязательность | Производитель значения | Смысл |
|---|---|---|---|
| `contract_id` | да | FastSearch | Константа `fastsearch.search-source-anchor`, различающая этот wire-контракт |
| `contract_version` | да | FastSearch | Строковая константа `1`; неизвестная версия не читается как v1 |
| `producer_id` | да | FastSearch | Константа `fastsearch`, а не сетевой адрес |
| `workspace_id` | да | `WorkspaceProfile` | Корреляция с допущенной областью FastSearch; не переносимая identity общего графа |
| `contour` | да | `SourceAnchorProvenanceLookupOutcome` | `documentation` или `code`, относящийся к записи принятого поколения |
| `root_id` | да | `SourceAnchorProvenanceLookupOutcome` | Типизированный логический корень FastSearch; требует явного сопоставления у потребителя |
| `relative_path` | да | lookup, сверенный с `CanonicalRecord::locator` | Нормализованный UTF-8 путь с `/`, без абсолютного корня, `.` и `..` |
| `selector` | да | lookup, сверенный с `CanonicalRecord::locator` | Закрытый tagged union: `kind` и точный payload по правилам ниже |
| `record_id` | да | `CanonicalRecord` | Устойчивая identity записи внутри FastSearch; не идентификатор узла FastGraph |
| `record_content_hash` | да | `CanonicalRecord` | Хэш канонического содержимого записи с указанным алгоритмом и версией |
| `raw_source_sha256` | да | `SourceAnchorProvenanceLookupOutcome` | 64 hex SHA-256 исходных байтов до снятия BOM и нормализации строк |
| `state_generation` | да | `SourceAnchorProvenanceLookupOutcome` | Поколение корпуса, по которому сформирован ответ |

`selector` кодируется только одним из вариантов:

- `{"kind":"markdown_heading","heading_path":[...],"start_byte":S,"end_byte":E}` - непустой упорядоченный массив непустых UTF-8 строк и точный полузамкнутый диапазон `[S,E)` в исходных байтах;
- `{"kind":"registry_row","row":N,"start_byte":S,"end_byte":E}` - положительный целый номер логической строки и её точный полузамкнутый диапазон в исходных байтах;
- `{"kind":"code_symbol","language":"...","symbol_kind":"...","qualified_name":"...","start_byte":S,"end_byte":E}` - непустые язык, вид и полное имя символа плюс точный полузамкнутый диапазон в исходных байтах;
- `{"kind":"whole_file"}` - без payload-полей.

`start_byte` является нулевым смещением, `end_byte` - первым байтом после диапазона; для непустой записи обязательно `0 <= start_byte < end_byte <= raw_file_length`. Смещения относятся к точным исходным байтам, подтверждённым `raw_source_sha256`, до снятия BOM и нормализации строк. Версия 1 сохраняет UTF-8 code points, регистр, пробелы и порядок `heading_path` ровно как в допущенном `CanonicalRecord`; Unicode, регистр и пробелы повторно не нормализуются. Текущая внутренняя строка code adapter вида `language:kind:name:start_byte` не является wire-форматом и должна быть разложена на структурные поля. Лишний, отсутствующий, пустой, выходящий за файл или не соответствующий `kind` payload даёт `UNSUPPORTED_SOURCE_ANCHOR`. В identity входят `contract_id + contract_version + producer_id + workspace_id + contour + root_id + relative_path + полный selector со смещениями + record_id + record_content_hash + raw_source_sha256 + state_generation`.

Абсолютный путь, SQLite-ключ, оценка модели, путь индекса и идентификатор узла FastGraph запрещены. Порядок результатов остаётся полем внешнего `PublicSearchResponse` и не входит в identity привязки.

## Преобразования и потери

Производитель удаляет машинный корень, но сохраняет его логическую замену `workspace_id + contour + root_id`. Он не передаёт ранжирующие оценки, внутренние ключи проекций и служебные пути: эти данные не нужны для разрешения источника. Потеря абсолютного пути допустима только потому, что потребитель обязан независимо допустить и сопоставить логический корень. Нормализованный `FileHash` не передаётся как raw hash и не считается потерей обещанного поля. Потеря `raw_source_sha256` или типизированного root запрещена; без них результатом является `BLOCKED / SOURCE_PROVENANCE_UNAVAILABLE`, а не ослабленная привязка. При финализации internal assembly/position wrapper заменяется внешним порядком outcomes и отдельным permit; эта потеря объявлена. Abort/failure сознательно уничтожают provisional payload: наружу передаются безопасный код, assembly identity и доступные generation/stage/cause, а внешний response не становится частичным.

## Цепочка поставки данных

`WorkspaceStore` публикует профиль -> входной адаптер читает исходные байты и до нормализации вычисляет raw SHA-256 -> канонизация отдельно создаёт нормализованный `FileHash`, запись и полный селектор -> опубликованное состояние атомарно связывает `record_id`, типизированный root/locator, raw hash и поколение -> поиск захватывает одно поколение и формирует `RankedCanonicalResultBundle` -> `application::public_search` INIT создаёт assembly state даже для нуля результатов -> lookup создаёт correlated provisional payload -> pre-permit termination либо полный state и acquire request -> coordinator после durable confirmation возвращает `ACTIVE_ACQUIRED|REPLAYED_TERMINAL|REPLAYED_PROCESS_DIED|CHANGED|FAILED` -> только active-ветвь передаёт `FinalizedAnchorAssembly + AnchorPublicationPermit`, terminal replay передаёт exact journal ref/digest, process-death replay - recovery-only outcome, остальные - один pre-lease terminal -> владелец Public Search v2 durable-outbox фиксирует единственный terminal или suppression -> внешний envelope идёт CLI/MCP только после commit/readback, а release controller независимо ведёт leased terminal до closure -> композиционный клиент извлекает опубликованную привязку -> FastGraph независимо допускает свой корень, проверяет сопоставление и raw hash и возвращает собственную identity узла либо типизированный отказ.

FastSearch создаёт поисковую привязку, но не графовый узел. Композиционный клиент создаёт последовательность вызовов, но не смысл полей. FastGraph создаёт результат разрешения и пути обхода, но не исправляет привязку. `cf-contracts` не участвует в этой цепочке данных: он нормализует документы контрактов для контрактного графа FastGraph, а не пользовательские поисковые результаты.

## Отказы и восстановление

`SOURCE_PROVENANCE_UNAVAILABLE` и `SOURCE_PROVENANCE_MISMATCH` создают per-result `BLOCKED` outcome и не уничтожают базовые поля результата. Generation mismatch и coordinator `FAILED` дают pre-lease failure только из domain outcome. `NOT_COMMITTED_RETRY|COMMIT_READBACK_REQUIRED` сохраняют `LEASE_PENDING` и уже записанный pending `CANCELLED|TIMEOUT` под той же acquire identity. `COMMIT_RECOVERY_BLOCKED` либо typed ledger-access block переводит ordinary и pending assembly в `RECOVERY_BLOCKED`: потенциально записанный acquire запрещает выдавать pending event как доказанно no-lease terminal. Lease recovery и writer block принадлежат отдельному lease-контракту.

Восстановление выполняется новым поиском после обновления корпуса либо явной настройкой сопоставления корней у потребителя. Исключение - exact `REPLAYED_TERMINAL`: thin_search обязан открыть уже committed terminal по `terminal_journal_ref`, проверить `terminal_record_digest + committed_terminal_identity` и повторить исходные immutable bytes либо исходный `SUPPRESSED`; отсутствующая или несовпадающая запись является typed lifecycle corruption, а не поводом собрать ответ заново из текущего поколения. Сам `application::public_search` не создаёт новый release. Если replay уже содержит authoritative `released_revision`, а response outbox всё ещё хранит ту же release identity как `RELEASE_PENDING|RECOVERY_REQUIRED`, thin_search создаёт только continuation исходной операции и повторяет exact same-key request до `ALREADY_RELEASED -> CLOSED`; новая release identity, новый permit и второй envelope запрещены. Состояние DT4 без provenance-индекса не дополняется частично: DT13-A поднимает версию локальной схемы, а `FS_SOURCE_PROVENANCE_REBUILD` под writer lock атомарно публикует `schema_marker + canonical generation + records + provenance index`. До commit действует прежний bundle; после commit и до отдельного `FS_SEARCH_REBUILD_PROJECTIONS` поиск остаётся `NOT_READY`, но смешанное поколение не принимается. Повтор старой привязки без совпадающего хэша запрещён.

## Условия передачи и приёмки

DT13-A может принять границу после реализации схем и независимого потребителя. Проверка дополнительно обязана покрыть трёхветочное взаимоисключение acquire domain outcome/commit-recovery status/state-access block; `NOT_COMMITTED_RETRY|COMMIT_READBACK_REQUIRED` без нового acquire identity; `COMMIT_RECOVERY_BLOCKED` и access block без permit/no-lease terminal/release; pending `CANCELLED|TIMEOUT` во время каждого recovery-исхода; передачу exact evidence supervisor. Остальная матрица сохраняет generation equality, zero-result INIT, concurrent assembly, single lifecycle CAS, stable permit, terminal/process-death replay и отсутствие direct outcome-to-wire path.

DT13-B начинается отдельно и только после принятого DT13-A, совместимого принятого выпуска DT4-E, канонической пары и выпуска FastGraph, которые называют этот формат входом, описывают сопоставление корней, проверку хэша, собственные выходы и отказные маршруты, а также после назначения композиционного клиента. Приёмка DT13-A не доказывает выпускную готовность любого продукта и не блокирует DT4-E.

## Контрактные и интеграционные испытания

Смысловой тест читает таблицу полей выше и проверяет сценарии: точное разрешение неизменного Markdown-раздела, два одинаковых полных heading path с разными byte ranges, два перегруженных symbol с одним именем и разными ranges, две registry row, одинаковый относительный путь в двух roots, изменение файла после ответа G1 и до потребления в G2, неизвестный корень, несовпадающий хэш и недопустимый абсолютный путь. Отдельные fixtures доказывают, что raw SHA различает BOM, LF, CRLF и CR даже при одинаковом текущем нормализованном `FileHash`, а старая схема состояния направляется на атомарный source/provenance rebuild, переживает crash до и после commit и не синтезирует raw hash. Машинная проверка читает [дескриптор](BC-FS-SEARCH-SOURCE-ANCHOR.boundary.json), проверяет одного производителя, происхождение каждого поля, закрытый handoff и отсутствие неразрешённых потерь.

## Блокеры

- В `PublicSearchResult v1` нет `SearchSourceAnchor`; требуется новая `PublicSearchResult v2`, атомарно несущая готовую привязку вместе с найденным результатом.
- Не реализован атомарный lookup `record_id + state_generation -> contour + root_id + locator + raw_source_sha256`; `CanonicalRecord` не содержит типизированный root.
- Не вычисляется и не сохраняется raw-byte SHA-256 до снятия BOM и нормализации строк; текущий `SourceSnapshot::file_hash` имеет другой смысл и формат.
- Не поднята версия disposable local state и не реализован атомарный `FS_SOURCE_PROVENANCE_REBUILD` для source/provenance bundle с последующим rebuild проекций того же поколения.
- Алгоритм и версия `record_content_hash` должны быть закреплены машинной схемой; текущий `ContentHash` принимает непрозрачную непустую строку.
- Не зафиксирован тестовый JSON Schema сериализуемого `SearchSourceAnchor v1`.
- FastGraph не опубликовал потребляющую пару и профиль разрешения источника.
- Не назначен выпускаемый композиционный клиент для DT13-B.
- Активная парадигма dtree всё ещё приписывает FastSearch документальный и кодовый граф; владелец dtree должен согласовать её с принятыми ролями `RetrievalProvider = FastSearch`, `GraphProvider = FastGraph` до выбора dtree композиционным клиентом.

## Машинные договоры и связи

- Машинный дескриптор: [BC-FS-SEARCH-SOURCE-ANCHOR.boundary.json](BC-FS-SEARCH-SOURCE-ANCHOR.boundary.json).
- Внешний wire-контейнер и negotiation: [BC-FS-PUBLIC-SEARCH-V2](BC-FS-PUBLIC-SEARCH-V2.md).
- Publication lease и writer admission: [BC-FS-GENERATION-PUBLICATION-LEASE](BC-FS-GENERATION-PUBLICATION-LEASE.md).
- Доступные внутренние основания: [`RootedSourceLocator`, `CanonicalRecord` и `SourceSnapshot`](../../../src/domain/record.rs).
- Производящая публичная граница: [`PublicSearchResult`](../../../src/application/public_search.rs).
- Техническое решение: [TDR-FS-2.9](<../TDR/TDR-FS-2.9 Поисковая привязка к FastGraph.md>).
- Этап: [DT13](<../Roadmap/09 DT13 Поисковая привязка к FastGraph.md>).
- Карта контрактов: [карта граничных контрактов](<00 Карта граничных контрактов.md>).
