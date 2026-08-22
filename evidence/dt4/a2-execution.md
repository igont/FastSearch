# Выполнение листа A2

Исполнительная база: `1da4fd01ed22bab6e673ff219e5169714242a89b`. Среда: Windows x86_64, `rustc 1.95.0`, `cargo 1.95.0`, CPython 3.13.14. Веса моделей находились только во внешних кэшах `%LOCALAPPDATA%/FastSearch/models` и `%USERPROFILE%/.cache/huggingface/hub`.

## G-QWEN

Официальные `config.json`, `tokenizer.json` и `model.safetensors` редакции `e61197ed45024b0ed8a2d74b80b4d909f1255473` сверены с SHA-256 из `fixtures/ts-dt4-01/model-manifest.json`. Независимый Python-эталон и Candle 0.11.0 обработали одни и те же 10 пар с закреплённым шаблоном, длиной 8192 и токенами `yes=9693`, `no=2152`. Полный порядок совпал, наибольшее расхождение вероятности равно `0.000002026558`.

Команды:

```powershell
& C:\Users\garig\.cache\fastsearch-dt4-oracle-py313\Scripts\python.exe scripts\dt4_qwen_oracle.py --model-root C:\Users\garig\.cache\huggingface\hub\models--Qwen--Qwen3-Reranker-0.6B\snapshots\e61197ed45024b0ed8a2d74b80b4d909f1255473 --cases evidence\dt4\fixtures\qwen-preflight-cases.json --output evidence\dt4\qwen-python-oracle.json
cargo run --release --locked --example dt4_qwen_preflight -- C:\Users\garig\.cache\huggingface\hub\models--Qwen--Qwen3-Reranker-0.6B\snapshots\e61197ed45024b0ed8a2d74b80b4d909f1255473 evidence\dt4\fixtures\ts-dt4-01\model-manifest.json evidence\dt4\fixtures\qwen-preflight-cases.json evidence\dt4\qwen-python-oracle.json evidence\dt4\qwen-preflight.json
```

Результат: `G-QWEN = PASS`. До открытия модели манифест сверяется со встроенным эталоном, а открытые после проверки файловые дескрипторы запрещают изменение и замену артефактов на всё время работы модели. Отрицательные тесты подтверждают отказ при изменённом манифесте или подменённом корне модели, блокировку изменения и замены во время работы модели, а также освобождение всех дескрипторов при ошибке. Размер и контрольная сумма выпуска дополнительно проверены в `executable-weight-check.json`; веса не включены ни в Git, ни в исполняемый файл.

```powershell
cargo test --locked --lib adapters::qwen_reranker::tests -- --nocapture
```

Результат отрицательных проверок: 4 пройдено, 0 ошибок; подробности находятся в `qwen-security-negative.json`.

## G-MCP

Реальный клиент `rmcp 3.1.4` запустил отдельный сервер через `stdio`, выполнил начальное согласование и один протокольный `search` с `working_search_invoked=false`. Отдельные проверки подтвердили границу версии, подавление позднего ответа после отмены и чистое завершение по EOF.

```powershell
cargo test --locked --test dt4_mcp_sdk_preflight -- --nocapture
```

Результат: 4 пройдено, 0 ошибок; `G-MCP = PASS`. Подробности находятся в `mcp-preflight.json`.

## TS-DT4-01 и ресурсная проба

Корпус содержит ровно 10 файлов, запрос с завершающим переводом строки, закреплённые редакции и контрольные суммы моделей, шаблон рабочей области и независимый Python-оракул. Оракул фиксирует по пять кандидатов каждого из трёх каналов векторного представления, то есть 15 входных слотов до удаления повторов. После объединения по стабильному идентификатору Qwen вычисляет вероятности, а отдельный публичный предел оставляет итоговые шесть результатов.

```powershell
& C:\Users\garig\.cache\fastsearch-dt4-oracle-py313\Scripts\python.exe -m pip install --dry-run --ignore-installed --require-hashes -r evidence\dt4\oracle\requirements.lock
& C:\Users\garig\.cache\fastsearch-dt4-oracle-py313\Scripts\python.exe scripts\dt4_oracle.py --fixture-root evidence\dt4\fixtures\ts-dt4-01 --output evidence\dt4\fixtures\ts-dt4-01\oracle.json
cargo run --release --locked --example dt4_embedding_resource_preflight -- arctic evidence\dt4\resource-arctic.json
cargo run --release --locked --example dt4_embedding_resource_preflight -- e5 evidence\dt4\resource-e5-large.json
cargo run --release --locked --example dt4_embedding_resource_preflight -- nomic evidence\dt4\resource-nomic.json
cargo run --release --locked --example dt4_queue_resource_preflight -- evidence\dt4\resource-queue.json
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\dt4_compose_resource_preflight.ps1
```

Для каждого канала векторного представления измерены холодный и тёплый проходы на входах 32, 256 и 1024 символа при пакетах 1 и 2. Отдельно измерены очередь ёмкостью 2 и ответы размером 1 КиБ, 64 КиБ и 1 МиБ. Это исходные наблюдения, допустимые пределы не назначены. Результат: `G-RESOURCE-PREFLIGHT@A2 = PASS`.

## G-BASE@A2

```powershell
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked --no-fail-fast
cargo build --release --locked
git diff --check
```

Все команды завершились с кодом 0. Полный набор тестов не имел ошибок; тесты, требующие отдельного локального кэша или тяжёлой загрузки всех моделей, остались штатно помечены `ignored`. Обязательные роли A2 были выполнены отдельными воспроизводимыми пробами. Результат: `G-BASE@A2 = PASS`.

## Термины

MCP здесь означает протокол взаимодействия с инструментальным сервером, `stdio` - обмен через стандартные потоки процесса, EOF - закрытие входного потока. Python-оракул - независимый эталонный расчёт, с которым сравнивается реализация на Rust.

Манифест модели - закреплённый список её файлов, размеров и контрольных сумм. Файловый дескриптор - удерживаемый процессом системный доступ к уже проверенному файлу, который запрещает его изменение или замену до закрытия модели.
