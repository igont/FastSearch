# FastSearch

Архитектурные парадигмы, TDR и подробные будущие graph stages находятся в [Obsidian](Obsidian/00%20Навигация.md). Root [ROADMAP](ROADMAP.md) остаётся источником последовательности и фактического состояния DT1–DT4; future documents не объявляют capability реализованной.

FastSearch — локальный CLI для индексации и поиска по документации и исходному коду. Основной human-interface работает с persistent рабочими областями; прежний three-path CLI сохранён как совместимый operator/machine surface.

## Рабочие области

Один системный FastSearch executable хранит небольшой machine-local каталог известных областей. Каждая область имеет один `.fastsearch` namespace, ровно два optional source contours — `documentation` и `code` — и любое число roots внутри каждого contour. Все document roots входят в единый document namespace, все code roots — в code namespace; stable `root_id` не допускает collisions одинаковых relative locators.

Human flow начинается с recent-workspace picker либо сразу открывает область по current directory. При создании пользователь указывает workspace root, подтверждает найденные source roots и выбирает одну embedding-модель из каталога. Ноль источников, только документация, только код и оба contour являются поддерживаемыми состояниями. Service path, database path и model root не запрашиваются.

Каждая workspace-сводка завершается framework-rendered блоком следующего шага. При актуальном индексе он явно показывает ввод обычного поискового текста и вертикальные переходы `/model`, `/compare`, `/index`, `/sources`, `/help`, `/exit`. При stale/degraded/not-configured состоянии первым показывается обязательное recovery-действие; приглашение поиска до готовности индекса не выводится.

При первом открытии области с источниками FastSearch автоматически загружает только выбранную модель и выполняет readiness probe. Startup flow не запускает индексацию. Активная модель сохраняется в `.fastsearch/workspace.toml`; `/model` показывает responsive-таблицу каталога, после которой модель можно выбрать следующим вводом одного номера (`1`–`5`). `/model info <N|slug>` открывает источник и технические сведения, а явная форма `/model set <N|slug>` также меняет выбор и требует последующего `/index update` или `/index rebuild`.

Workspace guidance зависит от lifecycle: только `CURRENT` предлагает обычный поисковый запрос; `STALE` предлагает `/index update`, `DEGRADED` — `/status` и `/index rebuild`, а отсутствие источников — `/sources set`. Bare text при неготовом индексе не запускает progress/search и возвращает typed `SEARCH_NOT_READY` с recovery command.

Обычные Markdown и TSV входят в document corpus. Производные coverage-реестры внутри каталога `Traceability` не допускаются в полнотекстовый и векторный индекс, если их TSV-схема содержит `path`, `summary`, `tdr_refs`, `warnings`, `errors` и колонку `*_coverage`: их текст уже присутствует в канонических Markdown-документах. Файлы не удаляются и остаются доступными внешним traceability-инструментам. Обычные TDR, alignment и прочие TSV-реестры продолжают индексироваться. После обновления FastSearch выполните `/index update`, чтобы удалить ранее сохранённые дубли из локальных проекций.

Нормативные источники текущего workspace/UI baseline:

- [рабочие области и interface paradigm](Obsidian/Paradigms/Архитектура/Рабочие%20области%20и%20интерфейс/00%20Рабочие%20области%20и%20интерфейс.md);
- [TDR-FS-2 — Workspaces и terminal UX](Obsidian/Docs/TDR/TDR-FS-2%20Workspaces%20и%20terminal%20UX.md);
- [terminal UI templates](Obsidian/Docs/UX/01%20Нормативные%20шаблоны%20интерфейса.md).

## Текущий быстрый старт

Запустите программу без аргументов из каталога проекта либо из любого другого каталога:

```powershell
.\fastsearch.exe
```

Если current directory уже находится внутри области с `.fastsearch/workspace.toml`, она откроется сразу. Иначе FastSearch покажет недавние области. Для новой области выберите `N`, укажите один root или нажмите Enter для текущего каталога, проверьте найденные источники и подтвердите создание.

После открытия вводите поисковый запрос обычной строкой. Для выхода используется `/exit`; ошибка одной команды не завершает интерактивный сеанс.

Внутри workspace создаётся:

```text
.fastsearch/
  workspace.toml          # portable configuration
  knowledge/curated/      # portable accepted knowledge
  local/                  # ignored, rebuildable indexes/state/cache
```

Файл `.fastsearch/.gitignore` исключает только `/local/`; configuration и curated knowledge не игнорируются. Legacy `.cfknowledge` и `.search` не используются как target storage и не удаляются автоматически.

## Сборка

Нужен установленный Rust с Cargo. Для отладочной сборки выполните в корне репозитория:

```powershell
cargo build --locked
```

Для обычной release-сборки дважды нажмите `build_fastsearch.bat` или запустите его из PowerShell:

```powershell
.\build_fastsearch.bat
```

Скрипт использует зафиксированный `Cargo.lock`, target-specific режим линкера MSVC `/Brepro` и копирует готовый `fastsearch.exe` в корень проекта. Cargo складывает общие артефакты FastSearch в соседний каталог `..\.cargo-target\FastSearch`, поэтому зависимости повторно используются во всех worktree. Исходный release-бинарник остаётся в `..\.cargo-target\FastSearch\release\fastsearch.exe`.

## Интерактивный режим

Запуск без аргументов и команда `fastsearch chat` предназначены для человека. Интерфейс строится как короткая state machine: `workspace picker → create/discovery → open/index transition → search → results/detail`.

После настройки доступны команды:

```text
/search <запрос>
/related <номер>
/sources
/sources discover
/sources set
/index
/index update
/index rebuild
/status
/workspace
/open <номер>
/next
/prev
/page <номер>
/repeat
/help
/exit
```

Интерактивный поиск автоматически использует сбалансированное объединение
доступных каналов. Пользователю не требуется выбирать технический профиль
ранжирования; запрос можно вводить обычным текстом без `/search`.

Команда `/sources` показывает оба contour, `/sources discover` повторяет automatic discovery, а `/sources set` позволяет задать несколько roots через `;` или отключить contour значением `-`. Поисковая выдача показывает по пять карточек на странице: номер, относительное совпадение в процентах, одну белую строку `Файл` с полным путём и выделенную триггерную фразу длиной до 165 символов без внутренних ID, retrieval channel и raw score. Процент нормализован относительно первого результата текущей выдачи (`100%`) и не является вероятностью или абсолютной оценкой качества. Выдача хранится до следующего поиска: `/open N` открывает результат по стабильному абсолютному номеру, `/related N` показывает связи, `/next`/`/prev` листают страницы, `/page N` переходит к странице, а `/repeat` повторяет текущую выдачу. `/workspace` возвращает к выбору области. `/index rebuild` всегда показывает preview и требует подтверждения; обычный search не скрывает full rebuild.

Текущий renderer уже поддерживает список из нескольких триггерных фраз на одну карточку. Paragraph-level retrieval, при котором каждый абзац независимо сравнивается с запросом и материализует собственные доказательства совпадения, отложен до завершения текущего UI/runtime refactor и этим изменением не вводится.

Human-интерфейс полностью строится через соседнюю библиотеку `terminal-dialogue`: типизированные welcome, selection, progress, result, empty-state и error documents, заголовки капсом, единый двухпробельный отступ, синий адаптивный timestamp-разделитель, единые prompts, `NO_COLOR`, terminal echo и boundary-проверка запрета ручного вывода. JSON остаётся отдельным direct machine-контрактом и не получает human-оформление.

## Прямой запуск

Запуск с аргументами остаётся строгим и подходит для скриптов, CI и диагностики. Пути передаются при каждом вызове:

```powershell
.\fastsearch.exe init <documents> <code> <service> [e5-root]
.\fastsearch.exe index update <documents> <code> <service> [e5-root]
.\fastsearch.exe index rebuild <documents> <code> <service> [e5-root]
.\fastsearch.exe search <documents> <code> <service> <balanced|current|design> <query> [e5-root]
.\fastsearch.exe get <documents> <code> <service> <stable-id> [e5-root]
.\fastsearch.exe related <documents> <code> <service> <stable-id> [e5-root]
.\fastsearch.exe status <documents> <code> <service> [e5-root]
```

Глобальную опцию `--json` можно поставить в любую позицию прямой команды до разделителя `--`. Она переключает успешный результат и ошибку в стабильный JSON-формат. Без `--json` сохраняется прежний технический текстовый stdout. Чтобы передать буквальное значение `--json` как запрос или путь, поставьте перед ним разделитель `--`.

Пример:

```powershell
.\fastsearch.exe index update C:\Work\docs C:\Work\project C:\Work\fastsearch-state
.\fastsearch.exe --json search C:\Work\docs C:\Work\project C:\Work\fastsearch-state balanced "правила восстановления"
```

Необязательный `e5-root` является operator override для диагностики и compatibility. Если он не передан, direct CLI автоматически использует общий qualified model cache и при необходимости восстанавливает его до выполнения команды.

## Локальные embedding-модели

Workspace использует ровно одну модель. Каталог предназначен для controlled comparison, а не для одновременного ансамбля:

| Slug | Source | Runtime | Размерность | Ориентировочная загрузка |
|---|---|---|---:|---:|
| `multilingual-e5-small` | [intfloat/multilingual-e5-small](https://huggingface.co/intfloat/multilingual-e5-small) | ONNX · CPU / DirectML probe | 384 | 0.49 GB |
| `multilingual-e5-base` | [intfloat/multilingual-e5-base](https://huggingface.co/intfloat/multilingual-e5-base) | ONNX · CPU / DirectML probe | 768 | 1.13 GB |
| `multilingual-e5-large` | [Qdrant/multilingual-e5-large-onnx](https://huggingface.co/Qdrant/multilingual-e5-large-onnx) | ONNX + external data · CPU / DirectML probe | 1024 | 2.25 GB |
| `qwen3-embedding-0.6b` | [Qwen/Qwen3-Embedding-0.6B](https://huggingface.co/Qwen/Qwen3-Embedding-0.6B) | Candle CPU | 1024 | 1.20 GB |
| `nomic-embed-text-v2-moe` | [nomic-ai/nomic-embed-text-v2-moe](https://huggingface.co/nomic-ai/nomic-embed-text-v2-moe) | Candle CPU | 768 | 1.92 GB |

В executable веса не встраиваются. Machine-local cache находится в:

- `%LOCALAPPDATA%\FastSearch\models\...` на Windows;
- `$XDG_DATA_HOME/fastsearch/models/...` либо стандартный user data directory на Unix;
- `<FASTSEARCH_HOME>/models/...`, если явно задан `FASTSEARCH_HOME`.

FastSearch загружает catalog revision напрямую в Hugging Face-compatible cache: минутная попытка имеет сетевой timeout, незавершённый `.download` сохраняется, а следующая из 24 попыток продолжает его через HTTP Range. Поэтому зависший CDN не требует перезапуска программы. После публикации FastEmbed переиспользует тот же cache. Для corporate mirror поддерживается `HF_ENDPOINT`. Исторический E5 Small immutable-provider contour с exact manifest сохранён для qualification tests; остальные варианты являются candidate runtimes и должны пройти одинаковый corpus benchmark до выбора нового default.

Каталог отдельно показывает фактическую готовность весов и локальные capability `CPU`/`GPU`. `CPU` поддерживается всеми catalog models. `GPU` имеет состояние `?` до machine-local inference probe, `✓` после валидного embedding и `—` при недоступном backend, несовместимости либо нехватке ресурсов. Проба выполняется после установки модели, не читает workspace corpus и сохраняется в `%LOCALAPPDATA%\FastSearch\models\<slug>\runtime-capabilities.toml`. Для E5 на Windows используется DirectML; Qwen/Nomic пока остаются Candle CPU, поэтому GPU для них не обещается без отдельного CUDA qualification.

Числовой прогресс загрузки принадлежит `terminal-dialogue`: framework строит адаптивную полосу, процент и счётчик, а FastSearch передаёт только измеренные bytes. Индексирование не стартует после загрузки модели.

Batch size выбирается evidence-first. Воспроизводимый инструмент `cargo run --release --example batch_benchmark -- <corpus-root>` берёт до 128 реальных текстовых файлов, прогревает один runtime и сообщает медиану трёх запусков, documents/second и process working set. Измерение E5 Small на текущем heterogeneous FastSearch corpus выбрало batch `1`: `40.78 docs/s`, около `0.95 GiB`; batch `64` дал `28.98 docs/s` и около `5.08 GiB`. Большие batch не являются автоматическим улучшением из-за padding до самого длинного текста.

Устройство можно задать вторым аргументом: `cpu` либо `gpu`; третий аргумент ограничивает batch list, например `gpu 8,16,32`. На GTX 1050 Ti лучший DirectML результат получен при batch `32`: `23.17 docs/s`; CPU batch `1` оказался быстрее в `1.76x`. GPU batch `64` снизился до `21.14 docs/s` и занял около `3694 MiB` из `4096 MiB` VRAM. Поэтому наличие GPU capability `✓` не означает автоматического выбора GPU: production policy должна опираться на machine-local benchmark.

Интерактивный интерфейс показывает model selection и typed progress только через `terminal-dialogue`. После `/model` следующий ввод номера является короткой формой `/model set <N>`; вне каталога число остаётся обычным поисковым запросом. Выбор сначала полностью загружает и проверяет candidate и лишь затем атомарно сохраняет её как основную; при ошибке прежний выбор остаётся активным. Установка модели не изменяет source state и не запускает incremental index, rebuild либо search. Смена модели инвалидирует совместимость vector projection через model identity; lexical/maps/symbol search остаётся доступным при ошибке provider. `--help` и `--version` модель не загружают.

`/experiment record <оценка>` записывает active model, последний query, hit count, latency и заметку в `.fastsearch/knowledge/experiments/embedding-models.jsonl`. Этот журнал portable и не попадает под `/local/` ignore; модельные cache и derived indexes остаются локальными. Обычная suite не скачивает веса; real-model qualification выполняется отдельными ignored/explicit acceptance-прогонами.

### Режим сравнения моделей

Обычный режим всегда использует одну модель, сохранённую в настройках workspace; выбирать её перед каждым запросом не требуется. Команда `/compare` открывает отдельный contour и без скачивания или индексации показывает readiness всех catalog models. `/update` после typed preview и подтверждения один раз актуализирует shared corpus, сохраняет готовые partitions и загружает/строит только недостающее. Обычная строка выполняет один query всеми готовыми моделями; lexical baseline показывается один раз, model top-K идут отдельными вертикальными блоками, а `/open A1` или `/open L1` открывает запись. `/back` возвращает в workspace и не изменяет active model.

Локальное хранение: `.fastsearch/local/index/vector/<model-slug>/<model-revision>/` с `manifest.toml`, `records.sqlite` и `vectors.bin`. Shared SQLite, document/code state и lexical index не дублируются. Partition повторно открывается после перезапуска только при совпадении model identity, runtime contract, dimension, corpus fingerprint/generation и record hashes. Responsive readiness table показывает для каждой модели отдельный статус индекса, точный размер committed partition и длительность последнего успешного построения; для отсутствующего индекса используются явные `—`. Comparison update сначала один раз reconciles shared corpus, затем строго последовательно строит stale/absent partitions в порядке catalog. Подробный механизм и обязательный terminal contract зафиксированы в `TDR-FS-2.5`.

Текущие границы qualification: автоматизированы storage round-trip, stale admission, read-only entry и terminal routing. Перед выбором нового default остаются обязательны real-model acceptance минимум на двух моделях, disk-space preflight и versioned сохранение полного comparison run; одиночный `/experiment record` не заменяет этот benchmark.

## Справка и версия

Справочные команды не открывают индекс и завершаются успешно:

```powershell
.\fastsearch.exe --help
.\fastsearch.exe --version
```

Для справки также доступны `-h` и `help`, для версии — `-V` и `version`.

## Вывод, цвета и коды завершения

Интерактивный режим выводит заголовки, понятные описания результатов и короткие подсказки. Обычный текст без начального `/` и `/search <запрос>` выполняют сбалансированный поиск. Технические ranking modes сохраняются только в прямом compatibility CLI для автоматизации и диагностики. Цвет применяется только при выводе в настоящий терминал; перенаправленный вывод и прямые команды не содержат ANSI-последовательностей. Переменная окружения `NO_COLOR` отключает цвет принудительно.

Прямой режим сохраняет стабильный контракт для автоматизации: результат печатается в stdout, ошибка — в stderr. `--json` не меняет это разделение потоков и код завершения. Основные коды:

- `0` — команда выполнена;
- `1` — ошибка открытия источника, состояния, индекса или выполнения команды;
- `2` — неверная команда или набор аргументов.

Пути с пробелами заключайте в кавычки. Запрос в прямом режиме является одним аргументом, поэтому фразу также нужно заключать в кавычки.

## Проверка

Перед публикацией изменений выполните:

```powershell
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
.\build_fastsearch.bat
```
