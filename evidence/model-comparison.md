# Model comparison implementation evidence

Дата проверки: 2026-08-16.

## Реализованный срез

- одна active model обычного workspace остаётся независимой от comparison contour;
- vector partitions разделены по `model-slug/model-revision` и публикуются через manifest-last commit;
- admission проверяет model identity, runtime contract, dimension, canonical generation, corpus fingerprint, record hashes и размер vector payload;
- `/compare` выполняет read-only readiness check;
- readiness `/compare` и каталог `/model` выводятся через framework-owned
  `TableDocument`: приложение передаёт ячейки, но не табуляции, пробелы или
  расчёт ширины;
- command guidance передаётся как ordered `ActionItem` collection: framework
  выводит каждую команду отдельной строкой; склеенные `/a · /b` подсказки
  удалены из workspace, sources, model, results и comparison flows;
- `/sources` объясняет, что не открывает отдельный режим, расшифровывает `-`,
  предоставляет `/status`, `/help`, `/exit` и скрывает Windows `\\?\` prefix;
  относительные и абсолютные пути используют единый platform separator;
- широкий терминал получает Unicode-aware колонки, узкий — автоматический
  вертикальный fallback без потери данных;
- `/model info <N|slug>` отделяет длинные source URL и технические поля от
  компактной сравнительной таблицы;
- `/update` требует typed preview/confirmation, один раз reconciles shared corpus и строит только отсутствующие или stale partitions;
- единый query возвращает shared lexical baseline и стабильные отдельные model blocks; ошибки моделей изолированы;
- `/open A1|L1`, `/status`, `/back` и `/exit` являются явными переходами nested router;
- comparison UI и основной console проверяются одним запретом manual terminal rendering.
- stale/degraded/not-configured workspace больше не приглашает вводить запрос:
  bare search блокируется до запуска progress и возвращает typed
  `SEARCH_NOT_READY` с однозначным следующим действием.

## Проверки

- `cargo test --locked` — pass; network/real-model acceptance tests остаются explicit ignored по своим prerequisites;
- `cargo clippy --all-targets --locked -- -D warnings` — pass;
- `cargo build --release --locked` — pass;
- `git diff --check` — pass;
- release `fastsearch.exe` SHA-256: `A9EAD36DDBEA2970BDE16C49F217D7F312E7780BC25FF62B03B97715EE302121`;
- graph postflight: 892 nodes, 8865 edges, 66 files; предыдущая оценка blast radius была high (383 nodes, 44 additional files), поэтому принят полный regression suite, а не только comparison tests.

## Метрики partitions

- manifest schema v2 сохраняет длительность последнего успешного build до commit marker;
- размер readiness вычисляет по фактическим committed-файлам partition;
- shared reconcile не строит active vector вне очереди: stale/absent models обрабатываются последовательно в `EmbeddingModelId::ALL` catalog order;
- schema v1 без build metrics классифицируется как stale и получает метрики после следующего `/update`.

## Незакрытые qualification gates

- cross-restart acceptance с минимум двумя реальными моделями и одним corpus;
- disk-space preflight до загрузки/build;
- versioned full comparison run с judgments и aggregate metrics;
- interruption/resume acceptance на реальном много-модельном build.

Эти пункты не препятствуют ручному `/compare`, но препятствуют объявлению новой default model без дополнительного evidence.
