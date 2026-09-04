---
TDR: TDR-FS-2.9
project_scope: future
document_status: actual
Индексация: true
---
# DT13. Поисковая привязка к FastGraph

## Место в последовательности и состояние

DT13 - условный интеграционный этап после стабилизации публичного ответа DT4. Он не входит в минимальный запуск FastSearch и не блокирует DT4-C, DT4-D или DT4-E. Этап разделён на производящий рубеж DT13-A и интеграционный рубеж DT13-B, чтобы работа FastSearch не ожидала ещё отсутствующий потребляющий контракт FastGraph.

## Вид и наблюдаемый результат

DT13-A - фундаментальный кандидат FastSearch из трёх канонических пар. [Public Search v2](<../Contracts/BC-FS-PUBLIC-SEARCH-V2.md>) задаёт явный выбор версии, durable response lifecycle outbox, сохранение базовых полей и exactly-one outcome; [SearchSourceAnchor v1](<../Contracts/BC-FS-SEARCH-SOURCE-ANCHOR.md>) задаёт точную identity и typed assembly; [Generation Publication Lease](<../Contracts/BC-FS-GENERATION-PUBLICATION-LEASE.md>) единолично задаёт active/replay state, writer admission и recovery. Каждый подходящий result v2 атомарно получает платформенно нейтральный outcome; независимый потребитель проверяет точный источник, полный payload селектора, хэши и поколение без доступа к внутреннему SQLite. Принятие A доказывает кандидатную публичную границу, но само по себе не является выпуском FastSearch.

DT13-B - пользовательский результат композиции. Пользователь находит материал через FastSearch и продолжает по зависимостям FastGraph; клиент показывает, какой продукт отказал и как восстановить цепочку. FastSearch при этом не строит граф, а FastGraph не выполняет полнотекстовое или векторное ранжирование.

## Основание и зависимости

DT13-A требует принятого DT4-D, [TDR-FS-2.9](<../TDR/TDR-FS-2.9 Поисковая привязка к FastGraph.md>) и согласованного смысла трёх целевых пар. Атомарное происхождение root/locator/raw-byte hash, его схема и rebuild являются результатами кандидата и условиями принятия DT13-A, а не входом для начала этапа. Реальный FastGraph для DT13-A не нужен.

DT13-B требует принятого DT13-A, принятого и выпущенного DT4-E с включённой совместимой v2 capability, канонической текстовой и машинной пары FastGraph для разрешения этого формата, выпускаемой версии FastGraph, явного сопоставления корней и назначенного композиционного клиента. Пока этих вводных нет, детальный план DT13-B недоступен.

## Контракты со старта

Сейчас можно согласовать владельца, состав и запреты `PublicSearchResponse v2`, `SearchSourceAnchor v1` и generation publication lease state, отсутствие прямой зависимости продуктов, обязанность FastGraph независимо допускать корень и запрет эвристического разрешения. Это зафиксировано в трёх парах `BC-FS-PUBLIC-SEARCH-V2`, `BC-FS-SEARCH-SOURCE-ANCHOR` и `BC-FS-GENERATION-PUBLICATION-LEASE`.

Сейчас нельзя согласовать точное имя capability-профиля FastGraph, его выходной DTO, коды всех внешних отказов и транспорт композиционного клиента. Эти решения принадлежат отсутствующей потребляющей паре FastGraph и будущему владельцу клиента; FastSearch не заполняет их предположениями.

## Технические решения DT13-A

Новый атомарный provenance lookup сохраняет прежнюю точную форму. Generation coordinator и thin_search outbox остаются разными authority. Coordinator возвращает для каждой state-changing операции ровно одну из трёх взаимоисключающих ветвей: domain outcome после `COMMITTED_EXACT`; generic `NOT_COMMITTED_RETRY|COMMIT_READBACK_REQUIRED|COMMIT_RECOVERY_BLOCKED`, если commit proposal не доказан; либо typed `GenerationLeaseStateAccessBlocked`, если авторитетный ledger потерян, повреждён или недоступен до proposal либо при восстановлении retained proposal. Ни commit status, ни access block не создают permit/release/closure. Response outbox владеет закрытым lookup `ABSENT|OPEN|TERMINAL_COMMITTED|PREOPEN_PROCESS_DIED_CLOSED|CORRUPT|UNAVAILABLE`: `UNAVAILABLE` не изображает `ABSENT`, любой pre-lease terminal сначала обязан пройти этот lookup, а pre-open tombstone не является public terminal. Две authority не имеют общей БД и не удерживают lock при обращении друг к другу. `PublicSearchResult v2` получает привязку при построении ответа того же поколения; wire-объект не раскрывает внутренние assembly/lease поля.

Pre-lease terminal создаётся только из authoritative `ABSENT`; `TERMINAL_COMMITTED` переигрывается с исходным `NONE|lease_id` scope и closure continuation, `OPEN` даёт `PRELEASE_OPEN_REPLAY_REQUIRED`, а приоритетные `CORRUPT|UNAVAILABLE` обслуживаются только generic reject path. Bounded outbox recovery имеет `RETRY_REQUIRED -> EXHAUSTED -> PublicSearchRecoveryBlocked`: outbox/acquire scope не создаёт terminal, release scope сохраняет уже committed terminal и блокирует лишь closure/writer. Release `RECOVERY_BLOCKED` хранится в outbox вместе с exact typed evidence; commit/readback и restart идемпотентно воспроизводят тот же control outcome без release retry. После restart domain outcome, commit-recovery status и state-access block используют `ReplayedGenerationLeaseContinuation` XOR initial terminal и authoritative row; redelivered coordinator response сам даёт restart intent и не требует finalized assembly/permit. Acquire `REPLAYED_TERMINAL(released_revision)` при ещё открытой closure продолжает только исходный same-key release до `ALREADY_RELEASED -> CLOSED`. Initial notification не переиздаётся. Только `RELEASED|ALREADY_RELEASED` доказывает `CLOSED`. Generation coordinator единолично создаёт ordered command на каждый removed lease; supervisor хранит delivery progress по `workspace identity + command_id` и открывает только outbox этого workspace, siblings и разные workspace закрываются независимо. Stale command после `CLOSED` получает `REJECTED_MISMATCH` acknowledgement с expected/actual evidence без перехода и без `APPLIED`; supervisor не завершает delivery progress. `PublicSearchError v2` имеет семь фиксированных code/owner/action отображений.

Первичный terminal commit и restart-readback машинно разделяет закрытый `ResponseLifecycleEntryMode`: `INITIAL_COMMIT_CONFIRMATION` XOR `RESTART_READBACK`, с proposal/commit либо детерминированной identity из authoritative terminal revision/digest, exact restart intent, process epoch и lifecycle entry attempt. Dispatcher производит mode внутри thin_search, получает его по явному flow-edge, а retained terminal row и redelivered response заменяют отдельный work item. Один authoritative lookup в одной попытке не может открыть обе ветви; все три обработчика coordinator response сверяют carrier с mode и встраивают полный mode payload в release-derived closure proposal. В той же epoch live-confirm требует exact match; после crash отдельный persisted-readback transform потребляет `TERMINAL_COMMITTED` с тем же самодостаточным evidence без старых outputs или скрытого хранилища и не допускает перепривязку новым mode. Недоказанный proposal остаётся recovery. Process-death closure использует command identity без mode; stable release identity не меняется.

DT13-A поднимает маркер схемы disposable local state. Typed lookup различает `NEVER_INITIALIZED|PRESENT|MISSING_PREVIOUS|CORRUPT|UNAVAILABLE`; только первый вариант допускает fresh bootstrap под exclusive lock из уже опубликованного generation, только второй - action transitions, остальные три приоритетно блокируют без proposal. Состояние DT4 без атомарного provenance-индекса не мигрируется частично и не получает синтетические raw hash. `FS_SOURCE_PROVENANCE_REBUILD` получает только durable persisted writer permit, атомарно публикует `schema_marker + canonical generation + records + provenance index`, затем возвращает typed commit/abort; coordinator меняет `state_generation` только после durable ledger readback. Единый commit-recovery status сохраняет operation/proposal identity для `NOT_COMMITTED|UNKNOWN` во всех operation kinds и не выдаёт предметный outcome. Crash до commit сохраняет старый bundle, crash после commit требует authoritative bundle reader и bounded recovery вплоть до terminal operator-owned stop. До `FS_SEARCH_REBUILD_PROJECTIONS` того же поколения поиск остаётся `NOT_READY`. Запрос без `response_version` продолжает выдавать v1; exact `2` допускается лишь при authoritative capability `V1_V2_READY`, неизвестная версия отклоняется до поиска.

## Технические решения DT13-B

Композиционный клиент сначала вызывает FastSearch, затем передаёт выбранную привязку FastGraph. FastGraph сопоставляет логические корни с собственным admission, проверяет `raw_source_sha256` по исходным байтам, закрепляет ревизию и возвращает собственную identity узла или отказ. Последующий обход выполняется GraphProvider.

Прямой вызов FastSearch -> FastGraph, общий индекс, общая SQLite, копирование графовых рёбер в FastSearch и title-only fallback запрещены. Повтор после изменения источника начинается с нового поиска или явного обновления графа, а не с безусловного повторного использования старой привязки.

## Приёмка DT13-A

Кандидат FastSearch на точной редакции и независимый эталонный клиент проходят положительные сценарии для Markdown heading, registry row, code symbol и whole file. Проверка сравнивает все обязательные поля с независимым чтением профиля и источника. Выпускная пригодность и чистая поставка остаются ответственностью DT4-E и повторно требуются перед DT13-B.

Негативная матрица дополнительно включает: capability-first negotiation; все семь error mapping; pre-lease `ABSENT|OPEN|TERMINAL_COMMITTED(NONE|lease_id)|PREOPEN_PROCESS_DIED_CLOSED|CORRUPT|UNAVAILABLE`; ровно один reject status для corrupt/unavailable; acquire/release domain-outcome XOR commit-status; взаимно исключающие initial-release и restart-resume selectors; restart-ответы `RELEASED|ALREADY_RELEASED|RELEASE_FAILED`, каждый release commit-recovery status и state-access block через replay-continuation carrier, включая crash до consume; `release committed -> crash до response-outbox CLOSED -> acquire REPLAYED_TERMINAL -> same-key ALREADY_RELEASED -> CLOSED`; closure proposal -> crash -> `NOT_COMMITTED|UNKNOWN` + opposite mode без перехода и persisted transition с полным embedded mode evidence; outbox/acquire block без terminal; `terminal committed -> release UNKNOWN|EXHAUSTED` с сохранённым terminal и незакрытым lease; запрет `CLOSED` из generic status; один coordinator fan-out batch на два lease, пустой batch, active lease -> owner death -> `ABSENT` tombstone, независимую ошибку command и lost-ACK replay `ProcessDiedClosureAck/APPLIED|ALREADY_APPLIED` для `ABSENT|OPEN|TERMINAL_COMMITTED`, одинаковые assembly/lease/command ID в двух workspace после restart supervisor и stale command после `CLOSED -> REJECTED_MISMATCH` acknowledgement с evidence без перехода и без `APPLIED`; supervisor delivery остаётся незавершённой. Сохраняются проверки поколений, корреляции, terminal winner, lost ACK, journal replay, writer recovery и raw-byte provenance.

Selector-fixture обязан доказать, что один `TERMINAL_COMMITTED` lookup и одна lifecycle entry attempt не создают одновременно initial/replay carrier либо два same-key transport release request. Crash-fixture после terminal commit, но до dispatch либо consume coordinator response, восстанавливает restart mode в новой epoch только из retained terminal revision/digest и exact restart intent. Release-derived closure commit в той же epoch принимает только byte-identical live mode; после crash отдельный transform принимает `TERMINAL_COMMITTED` persisted transition для `CLOSED|RECOVERY_REQUIRED|RECOVERY_BLOCKED` по полному embedded mode evidence без старых outputs, другого lookup или новой перепривязки. Process-death closure отдельно принимается по command identity.

## Приёмка DT13-B

Независимый клиент использует два выпускаемых бинарника и только публичные поверхности. Неизменный источник разрешается в ожидаемый узел закреплённой ревизии и допускает один графовый обход. Несовпадающий хэш, отсутствующее сопоставление корня, неподдерживаемая версия и отсутствующий узел возвращают разные отказные результаты с названным владельцем восстановления.

Приёмка повторно проверяет контракт FastSearch, потребляющий контракт FastGraph и клиентский handoff на точных редакциях. PASS одного продукта не заменяет PASS всей цепочки.

## Параллельные фронты

| Фронт | Можно начинать | Чего ждёт | Что разблокирует |
|---|---|---|---|
| DT4-C/D/E | сейчас по своей последовательности | только собственные принятые рубежи FastSearch | минимальный Windows-выпуск поиска |
| `cf-contracts` | независимо от FastSearch | свои правила и производители контрактных документов | `ContractRegistrySnapshot` для FG2C |
| FastGraph FG2C | после своей внешней пары `cf-contracts` | не ждёт DT13 и FastSearch | контрактный граф и его GraphProvider-профиль |
| DT13-A candidate | после принятого DT4-D | принятый TDR-FS-2.9 и согласованный смысл трёх целевых пар; их runtime ещё не требуется | реализованные и принятые пары, response outbox, provenance lineage/rebuild и кандидатную границу FastSearch, но не выпуск |
| Потребляющий профиль FastGraph | параллельно DT13-A после согласования входного формата | собственную TDR и машинную пару | возможность планировать DT13-B |
| DT13-B | только после принятого DT13-A, выпущенного DT4-E и принятого профиля FastGraph | совместимые выпуски FastSearch и FastGraph, root mapping и композиционный клиент | поиск -> разрешение -> обход |

Активная парадигма dtree пока содержит дальнюю конфликтующую формулировку о документальном и кодовом графе FastSearch. Это не блокирует DT4 или DT13-A, но dtree нельзя назначить владельцем композиции DT13-B, пока его authority не приведена к принятому разделению `RetrievalProvider = FastSearch`, `GraphProvider = FastGraph`, `composition = dtree`.

## Не входит

Индексация контрактов, генерация `ContractRegistrySnapshot`, графовые рёбра, ревизии FastGraph, общий физический индекс и перенос GraphProvider в FastSearch не входят в DT13. DT12 также не используется: привязка к одному найденному материалу не является внешним снимком рабочей области.

## Пересмотр вдаль

После каждой принятой ветки DT4, `cf-contracts`, FastGraph FG2C, DT13-A и потребляющего профиля FastGraph повторно проверяются состав полей, авторитетные источники, root mapping, потери, версии и следующий владелец. Новая возможность FastGraph или изменение публичного ответа FastSearch не принимаются до оценки влияния на уже согласованную пару.
