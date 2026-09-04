---
TDR: TDR-FS-2; TDR-FS-2.6; TDR-FS-2.8; TDR-FG-1.10
project_scope: future
document_status: actual
Индексация: true
---
# TDR-FS-2.9 - Поисковая привязка к FastGraph

## Контекст и задача

FastSearch находит и ранжирует исходный материал, а FastGraph владеет графовыми узлами, рёбрами, ревизиями и обходом зависимостей. Совместный пользовательский маршрут должен позволить начать с поискового результата и продолжить по графу без общего хранилища, повторного эвристического поиска и переноса графовой ответственности в FastSearch.

Отдельно развивается `cf-contracts` - генератор, нормализатор и валидатор контрактов. Его снимок нужен контрактному профилю FastGraph, но не поисковому ответу FastSearch. Требуется развести два потока данных и назначить готовность каждого владельца.

## Поддерживаемые парадигмы

[FastSearch как поисковый контур](<../../Paradigms/Архитектура/02 FastSearch как поисковый контур.md>) оставляет retrieval основной ответственностью продукта. [Передача графового контура](<../FastGraph.md>) закрепляет независимость хранилищ. [TDR-FS-2.8](<TDR-FS-2.8 Модульные границы и контракты.md>) требует отдельной пары для передачи другому владельцу. FastGraph TDR-FG-1.10 оставляет GraphProvider единственной внешней графовой поверхностью.

## Входы и результаты

Входом решения являются существующие `RootedSourceLocator`, `CanonicalRecord`, `WorkspaceProfile`, публичный поисковый ответ и внешняя граница GraphProvider. Результат - [контракт поисковой привязки](<../Contracts/BC-FS-SEARCH-SOURCE-ANCHOR.md>), условный [DT13](<../Roadmap/09 DT13 Поисковая привязка к FastGraph.md>) и явная композиция двух независимых продуктов.

## Решение

FastSearch производит только `SearchSourceAnchor v1` и удерживает generation publication lease до durable consume-once terminal. Generation coordinator для каждой state-changing операции возвращает ровно одну из трёх взаимоисключающих ветвей: domain outcome после `COMMITTED_EXACT`; generic `NOT_COMMITTED_RETRY|COMMIT_READBACK_REQUIRED|COMMIT_RECOVERY_BLOCKED`, если commit proposal не доказан; либо typed `GenerationLeaseStateAccessBlocked`, если authoritative ledger потерян, повреждён или недоступен до proposal либо при восстановлении retained proposal. Ни commit status, ни access block не доказывают предметный результат. Response lifecycle outbox имеет закрытый lookup `ABSENT|OPEN|TERMINAL_COMMITTED|PREOPEN_PROCESS_DIED_CLOSED|CORRUPT|UNAVAILABLE`; pre-lease terminal создаётся только из authoritative `ABSENT`, а pre-open process-death tombstone не создаёт public terminal. Два хранилища не объединяются и не удерживают lock при обращении друг к другу. Привязка не содержит идентификатор узла FastGraph и не обещает наличие узла.

FastGraph в будущем публикует собственный профиль разрешения привязки. Он независимо допускает корень, сопоставляет логический корень FastSearch со своим `root_id`, проверяет `raw_source_sha256` по исходным байтам, закрепляет графовую ревизию и создаёт собственный результат разрешения или типизированный отказ. Эта потребляющая граница не принадлежит FastSearch и пока отсутствует.

Композиционный клиент выполняет два вызова: получает поисковый результат FastSearch с уже созданной неизменяемой привязкой, затем передаёт её в FastGraph и запрашивает обход. Клиент владеет последовательностью и пользовательским представлением, но не изменяет поля и не становится источником истины.

Продукты не получают прямой сборочной зависимости и не читают физическое хранилище друг друга. Отсутствие FastGraph не блокирует поиск, а отсутствие FastSearch не блокирует точное разрешение и обход уже известного идентификатора FastGraph.

## Два независимых потока данных

Пользовательский поток: `источник -> FastSearch admission/canonicalization -> поисковые проекции -> SearchSourceAnchor -> композиционный клиент -> FastGraph source resolver -> GraphProvider traversal`.

Контрактный поток: `владелец модуля -> текстовая карточка + машинный дескриптор -> cf-contracts ContractRegistrySnapshot -> транспорт FastGraph FG2C -> контрактные узлы и рёбра -> GraphProvider contracts profile`.

FastSearch не индексирует `ContractRegistrySnapshot`, не проверяет семантику контрактного графа и не хранит зависимости контрактов. Если контрактные документы входят в обычный допущенный корпус, FastSearch может находить их как текст, но такой результат не считается проверенным графовым фактом.

## Состояние до и после

До решения поисковый ответ содержит абсолютный путь, а идея «FastSearch индексирует представления FastGraph» допускает дублирование графовой проекции. После решения [Public Search v2](<../Contracts/BC-FS-PUBLIC-SEARCH-V2.md>) сохраняет базовые поля v1, включая путь только для локального показа, и атомарно добавляет exactly-one anchor outcome. Целевая межпродуктовая identity находится только в привязке и заблокирована до сквозного происхождения файла. Графовые представления, зависимости и ревизии остаются в FastGraph; FastSearch выдаёт лишь точку входа к источнику.

## Ошибки и восстановление

FastSearch не создаёт привязку при отсутствии или расхождении authority-backed provenance. При explicit v2 capability проверяется до base readiness: `V1_ONLY` даёт `PUBLIC_SEARCH_V2_NOT_READY`, а только `V1_V2_READY` открывает закрытую карту семи public error codes. Commit ambiguity acquire оставляет lifecycle в `LEASE_PENDING|RECOVERY_BLOCKED` без permit и без no-lease terminal. Post-permit termination применяется только к совпавшим permit, assembly и lease; все кандидаты входят в один outbox CAS. Во всех ветвях partial response запрещён.

Outbox recovery различает `RETRY_REQUIRED` и supervisor input `EXHAUSTED`; `CORRUPT|UNAVAILABLE` имеет один приоритетный generic reject path. Для outbox/acquire recovery public terminal отсутствует до доказанного commit; release recovery сохраняет уже committed terminal и блокирует только closure/writer. Release `RECOVERY_BLOCKED` атомарно хранит exact typed evidence в response outbox; `COMMITTED_EXACT` и restart-readback повторно создают byte-identical `PublicSearchRecoveryBlocked`, поэтому crash до handoff не теряет причину. Release domain outcome и generic commit status взаимоисключающие; `CLOSED` создаётся только из `RELEASED|ALREADY_RELEASED`. После restart любой повторно доставленный ответ release-координатора сам становится exact restart intent и вместе с retained terminal создаёт `ReplayedGenerationLeaseContinuation` без finalized assembly/permit; initial terminal не переиздаётся. Acquire `REPLAYED_TERMINAL(released_revision)` при ещё не закрытом response outbox продолжает только исходный same-key release до `ALREADY_RELEASED -> CLOSED`, поэтому уже выполненный release не оставляет orphan `RELEASE_PENDING`. Только generation coordinator строит ordered cleanup batch и stable commands; supervisor хранит delivery progress и повторяет прежний `workspace identity + command_id`, не выполняя второй fan-out и не открывая outbox другого workspace. Stale command после `CLOSED` даёт `REJECTED_MISMATCH` acknowledgement с expected/actual evidence, но без перехода и без `APPLIED`; supervisor не завершает delivery progress. Operator repair authority в v1 не определена.

Первичный commit и restart-readback carrier разделяет отдельный закрытый `ResponseLifecycleEntryMode`: `INITIAL_COMMIT_CONFIRMATION(proposal, commit_attempt)` XOR `RESTART_READBACK(authoritative terminal revision/digest, deterministic restart identity, restart_intent_ref)`, оба с process epoch и lifecycle entry attempt. Dispatcher производит ровно один mode только для live terminal confirmation либо existing-terminal carrier reconstruction; persisted-closure readback и command-only process-death replay производят ноль live mode. Retained terminal row и redelivered response служат authority без отдельного work item. Mode имеет явное flow-edge в dispatcher, запрещает применить confirm и carrier-readback к одному lookup в одной попытке, проверяется всеми тремя обработчиками ответа release-координатора и целиком встраивается как `response_entry_mode_evidence` в release-derived closure proposal; digest покрывает каждое поле. После crash отдельный persisted-readback transform получает то же полное значение внутри `TERMINAL_COMMITTED`, без эфемерных outputs старой epoch и отдельного lookup; новый opposite mode не может его перепривязать. Недоказанный commit без прежнего mode остаётся recovery. Process-death proposal вместо mode несёт exact command identity. Mode не входит в stable release identity.

Смена любого публичного поля создаёт новую версию контракта. Старая версия не перенаправляется молча на «последнюю» ревизию.

## Инварианты

- Поиск остаётся полезным и принимаемым без FastGraph.
- Графовое разрешение и обход остаются полезными без FastSearch при известной identity FastGraph.
- `record_id` FastSearch никогда не трактуется как `node_id` FastGraph.
- Абсолютный путь не является межпродуктовой identity.
- Один `SearchSourceAnchor` относится к одному авторитетному поколению и одному хэшу файла.
- `response_assembly_id + position_id` изолируют provisional payload; только `FinalizedAnchorAssembly` передаёт outcomes outer producer, прямой outcome-to-wire path запрещён.
- Publication lease удерживает generation от проверки до единственного terminal; active replay возвращает тот же lease, terminal/process-death replay возвращает отдельный outcome без нового permit, одна полная release identity повторяется controller/supervisor до closure, а writer допускается только после durable empty-active-set decision и завершает сохранённый permit typed commit/abort/recovery.
- Response lifecycle outbox является единственным источником истины terminal/delivery/release/token/closure; normal response не обходит termination arbitration, а потеря ACK не создаёт второй identity.
- Полный selector payload и версия входят в wire identity; `record_id`, path и rank не заменяют их.
- DT13-A поднимает версию disposable local state; отдельный `FS_SOURCE_PROVENANCE_REBUILD` атомарно публикует source/provenance bundle, после чего проекции перестраиваются для того же поколения. Частичное обогащение старого состояния запрещено.
- Сопоставление корней явно настраивает потребитель; совпадение имени не является доказательством.
- `cf-contracts` производит нормализованный контрактный снимок, FastGraph - графовые факты, FastSearch - поисковую привязку, клиент - последовательность вызовов.

## Готовность и параллельность

| Состояние | Необходимые факты | Что разрешает |
|---|---|---|
| `DT4_E_READY` | приняты DT4-C и DT4-D; точная кандидатная редакция готова к выпускной проверке | независимый минимальный Windows-выпуск без DT13 |
| `DT13_A_CANDIDATE` | принят DT4-D; согласованы три FastSearch-пары; реализованы provenance lineage/rebuild, generation ledger, response outbox, schema и независимый consumer | кандидатную приёмку Public Search v2, но не выпуск продукта |
| `DT13_B_READY_FOR_PLANNING` | принят DT13-A; принят и выпущен совместимый DT4-E; FastGraph опубликовал принятую потребляющую пару и выпуск; назначены root mapping и клиент | детальный план композиции |
| `DT13_B_ACCEPTED` | два точных выпуска и независимая клиентская E2E-приёмка повторно подтвердили обе пары | пользовательский маршрут search -> resolve -> traverse |

DT4-C, DT4-D и DT4-E выполняются без ожидания FastGraph и `cf-contracts`. После принятого DT4-D можно параллельно строить `DT13_A_CANDIDATE`: реализовать Public Search v2, переговоры версий, атомарное происхождение root/locator/byte-range/raw hash, source/provenance rebuild, generation ledger, response outbox, JSON Schema и независимого эталонного потребителя. `V1_V2_READY` становится выпускаемой capability только в принятом выпуске, а не из одного факта кандидатной приёмки A.

FastGraph FG2C может проектировать контрактный граф после появления канонической пары и снимка `cf-contracts`; FastSearch для этого не требуется. Реальный DT13-B ждёт одновременно принятого DT13-A, совместимого принятого выпуска DT4-E, канонической потребляющей пары и выпуска FastGraph, явного сопоставления корней и выбранного композиционного клиента.

## Рассмотренные варианты

Индексировать графовые представления FastGraph внутри FastSearch не принято: это создаёт вторую ревизию графовой истины, отдельную актуализацию и неоднозначный владелец зависимостей.

Передавать только абсолютный путь не принято: путь платформенный, раскрывает локальную топологию и не защищает от разрешения другой редакции файла.

Передавать только `record_id` не принято: его пространство принадлежит FastSearch и не доказывает соответствие узлу или ревизии FastGraph.

Встроить генератор контрактов в FastGraph не принято: нормализация должна быть независимой от конкретного хранилища и пригодной для строгой проверки до импорта. Встроить его в FastSearch также не принято, потому что поисковый продукт не владеет семантикой контрактов.

## Последствия

Появляется небольшой общий формат привязки и один внешний профиль FastGraph вместо общего индекса. Цена решения - явное сопоставление корней, атомарный provenance lookup с raw-byte hash и дополнительный клиентский вызов. Зато каждый продукт сохраняет автономный выпуск, источник истины и понятный отказ.

## Проверка решения

Механическая проверка сопоставляет карточку и дескриптор; семантическая доказывает достижимость, взаимоисключение, кардинальность, authority и recovery. Обязательные новые fixtures: все семь error mappings; capability-first negotiation; pre-lease lookup matrix; acquire/release domain-outcome XOR commit-status; mutually exclusive initial-release/restart-resume selectors; restart-ответы `RELEASED|ALREADY_RELEASED|RELEASE_FAILED`, каждый release commit-recovery status и state-access block через replay-continuation carrier, включая crash до consume; `release committed -> crash до response-outbox CLOSED -> acquire REPLAYED_TERMINAL -> same-key ALREADY_RELEASED -> CLOSED`; closure proposal -> crash -> `NOT_COMMITTED|UNKNOWN` + opposite mode с нулём переходов и отдельный persisted readback с проверкой полного embedded mode evidence; `RETRY -> EXHAUSTED` без fabricated terminal; запрет `CLOSED` из generic status; ordered multi-lease process-death fan-out, пустой batch, active lease -> `ABSENT` pre-open tombstone, lost-ACK replay durable `ProcessDiedClosureAck` для `ABSENT|OPEN|TERMINAL_COMMITTED`, двух-workspace collision через restart supervisor и stale command после `CLOSED -> REJECTED_MISMATCH` без transition. Остальные проверки покрывают generation/correlation, terminal races, lost ACK, journal replay, writer recovery, raw-byte provenance и независимого потребителя.

Selector-fixture отдельно доказывает, что один `TERMINAL_COMMITTED` lookup и одна lifecycle entry attempt не могут породить одновременно `PublicResponseAssemblyTerminal`, `ReplayedGenerationLeaseContinuation` либо два same-key transport request. Crash-fixture после terminal commit, но до dispatch или consume coordinator response, обязан в новой epoch детерминированно восстановить restart mode из retained terminal revision/digest и exact restart intent. Для release-derived closure proposal `NOT_COMMITTED|UNKNOWN` с противоположным либо отсутствующим mode не меняет closure; authoritative `TERMINAL_COMMITTED` с persisted transition отдельно восстанавливает `CLOSED|RECOVERY_REQUIRED|RECOVERY_BLOCKED` только по полному embedded mode evidence, без старых outputs, другого lookup и нового live mode. Process-death closure отдельно принимается по command identity без response mode.

Интеграционная приёмка DT13-B использует два выпускаемых бинарника и независимый клиент: результат FastSearch разрешается в точный узел закреплённой ревизии FastGraph; изменение файла, неизвестный root mapping и отсутствие узла возвращают разные типизированные отказы. Проверка не читает внутренние SQLite обоих продуктов.

## Отложенные вопросы

Точная JSON Schema строится в составе DT13-A и закрывается до принятия его кандидатной редакции. Имя FastGraph capability-профиля закрывается до DT13-B. Способ поставки композиционного клиента выбирается после подтверждения владельца пользовательского маршрута; он не влияет на ownership данных.

## Термины

Поисковая привязка - проверяемое описание исходного материала, найденного FastSearch. Разрешение привязки - принадлежащая FastGraph операция сопоставления материала с узлом закреплённой ревизии. Контрактный снимок - нормализованный выход `cf-contracts`, не связанный с поисковым результатом.
