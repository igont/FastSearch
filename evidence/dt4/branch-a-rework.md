# Корректирующая проверка ветки A

## Результат

Замечания `A-BR-MR-001` и `A-BR-MR-002` закрыты на корпусе `TS-DT4-01`. Независимый оракул на Python использует структурный вход верхнеуровневого фрагмента `title + "\n" + body`, принятые префиксы моделей и первые пять результатов каждой проекции. Изолированный проверочный тест точно сопоставляет с ним 15 упорядоченных слотов, дедупликацию по первому появлению и публичный порядок. Вероятности Qwen сопоставляются по устойчивому идентификатору с допуском `0.00001`.

Оракул создан независимой реализацией моделей на Python, а наблюдение выполнено производственной реализацией на Rust. SHA-256 скрипта: `442771f194446c2d9351b58bf14be8f73df313269d4f7fd7cd2e06951ebfd5c8`. SHA-256 оракула: `34ef540e6f27dc922ee45e82affbaf7b39ea20b04c2942a5eabbdc645a12f5d2`. Допуск Qwen превышает независимо измеренное на этом корпусе расхождение Python и Candle `0.000002026558`.

Изолированный MCP-каркас проверяет официальный клиент `rmcp` через исполняемый файл выпуска, протокол `2025-11-25`, публичную шестёрку, отмену без позднего ответа, EOF и `BUSY` для третьего вызова. Каждый сценарий создаёт отдельные рабочую область и `FASTSEARCH_HOME`, удаляет из окружения дочернего процесса четыре тестовые переменные и прежние `FASTSEARCH_A5_*`, не скачивает модели и не индексирует корпус внутри сервера. Каркас сверяет неизменность исходного кэша, глобальных каталогов FastSearch и Hugging Face, а также продуктовых файлов репозитория во время работы дочернего процесса. Операционные каталоги Git, динамического дерева, графа кода и сборки в эту проверку не входят.

Управляющие операции имеют отдельные временные границы. Инициализация клиента `rmcp`, начальный обмен `initialize`, завершение по EOF и ожидание дочернего процесса ограничены 10 секундами; ответ `BUSY` ограничен 2 секундами. Тяжёлый поиск и окно подавления отменённого ответа сохраняют наблюдательный предел 35 секунд. Отдельный быстрый тест фиксирует эти четыре значения и не зависит от точности системных часов.

Ресурсное наблюдение основного сценария записано в `evidence/dt4/resource-thin-slice.json`: основной сквозной MCP-сценарий до получения ответа занял 9 051 мс, пик рабочего набора составил 6 355 914 752 байта, до запуска было свободно 23 809 114 112 байт, публичный ответ занял 3 402 байта.

## Воспроизведение

Блок выполняется из корня репозитория в PowerShell. Он создаёт временный кэш из уже подготовленных закреплённых моделей, готовит проекции до запуска сервера и задаёт каркасу ровно четыре переменные `FASTSEARCH_DT4_*`. Повторный запуск пишет ресурсный результат под `$runRoot` и не изменяет отслеживаемые файлы кандидата.

```powershell
$runRoot = Join-Path $env:TEMP ("fastsearch-dt4-repeat-" + [guid]::NewGuid().ToString("N"))
$fixture = Join-Path $runRoot "fixture"
$modelCache = Join-Path $runRoot "model-cache"
$prepareEvidence = Join-Path $runRoot "prepare.json"
$resourceEvidence = Join-Path $runRoot "resource.json"
New-Item -ItemType Directory -Path $runRoot | Out-Null
Copy-Item -LiteralPath "evidence/dt4/fixtures/ts-dt4-01" -Destination $fixture -Recurse

cargo build --locked --release --bin fastsearch --example dt4_prepare_isolated_cache --example dt4_a5_prepare_fixture
& "../.cargo-target/FastSearch/release/examples/dt4_prepare_isolated_cache.exe" (Join-Path $fixture "model-manifest.json") $modelCache
$previousProductHome = [Environment]::GetEnvironmentVariable("FASTSEARCH_HOME", "Process")
try {
    $env:FASTSEARCH_HOME = $modelCache
    & "../.cargo-target/FastSearch/release/examples/dt4_a5_prepare_fixture.exe" $fixture (Join-Path $fixture "prepared-workspace") $prepareEvidence
} finally {
    if ($null -eq $previousProductHome) {
        Remove-Item Env:FASTSEARCH_HOME -ErrorAction SilentlyContinue
    } else {
        $env:FASTSEARCH_HOME = $previousProductHome
    }
}

$env:FASTSEARCH_DT4_FIXTURE_ROOT = $fixture
$env:FASTSEARCH_DT4_MODEL_CACHE = $modelCache
$env:FASTSEARCH_DT4_ORACLE = (Resolve-Path "evidence/dt4/fixtures/ts-dt4-01/oracle.json").Path
$env:FASTSEARCH_DT4_EVIDENCE_OUT = $resourceEvidence

cargo test --locked --lib -- --ignored --exact application::unified_search::tests::three_projection_candidates_match_independent_oracle
cargo test --locked --test dt4_mcp_thin_slice control_plane_windows_are_bounded_without_shortening_heavy_search -- --exact
cargo test --locked --test dt4_mcp_thin_slice -- --ignored --test-threads=1
```

Промежуточная команда выполнила `1/1` тест за 13,71 с. Быстрый тест временных границ выполнил `1/1` проверку без измеримого времени. Последовательный прогон MCP выполнил `4/4` сценария за 194,62 с; тяжёлые тестовые сценарии при этом не выполнялись одновременно.

## Термины

- `MCP` - протокол взаимодействия клиента с сервером инструментов.
- `initialize` - начальный обмен MCP для согласования версии протокола и возможностей сторон.
- `rmcp` - официальная библиотека Rust для MCP.
- `Qwen` - модель повторного ранжирования кандидатов.
- `Candle` - библиотека Rust, используемая для выполнения и проверки Qwen.
- `EOF` - завершение входного потока.
- `BUSY` - публичная ошибка при занятой активной и ожидающей позициях очереди.
- `Устойчивый идентификатор` - постоянный ключ фрагмента корпуса, используемый при сопоставлении и удалении повторов.
