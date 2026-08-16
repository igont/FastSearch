# Automatic E5 provisioning — acceptance evidence

Дата: 2026-08-16.

## Квалифицированный artifact

- Repository: `intfloat/multilingual-e5-small`.
- Revision: `614241f622f53c4eeff9890bdc4f31cfecc418b3`.
- Runtime files: `onnx/model.onnx`, `tokenizer.json`, `config.json`, `special_tokens_map.json`, `tokenizer_config.json`.
- ONNX SHA-256: `CA456C06B3A9505DDFD9131408916DD79290368331E7D76BB621F1CBA6BC8665`.
- Minimal manifest root: `8FCC7E28D97B8DA292E14631A6B46E03DD0890A4DA2AE244BE62813BC8CE53A6`.

## Clean-cache smoke

Актуальный release binary был запущен без `e5-root` при отсутствии `%LOCALAPPDATA%\FastSearch\models`. Runtime самостоятельно загрузил pinned model, проверил manifest и опубликовал revision root. Для проверки готовности затем была явно запущена direct-команда `search automatic embedding model`; direct search по своему контракту выполнил reconciliation и запрос.

Наблюдаемый результат:

```text
freshness=Current
hits=2
... channel=Vector ... automatic_vector_pipeline ...
... channel=Vector ... Vector retrieval ...
```

Model path и отдельная install-команда не передавались. Индексация в этом evidence относится к явно запрошенному direct search, а не к завершению установки модели. Отдельный CLI UX regression test подтверждает отсутствие implicit index при открытии workspace.

## Проверки

- `cargo test --all-targets` — обычный offline contour проходит; real E5 tests остаются ignored.
- `FASTSEARCH_E5_MODEL_ROOT=<auto-cache> cargo test --all-targets -- --ignored` — проходят immutable-provider security tests, deterministic vector lifecycle и configured-provider recovery.
- `cargo test --test e5_auto_pipeline -- --ignored` — materialized end-to-end provisioning/index/search test; search обязан содержать `RetrievalChannel::Vector`.
- Release smoke использует тот же `ProductionRuntime`, что human/direct product surfaces.

## Остаточные ограничения

- First download требует сети либо доступного `HF_ENDPOINT` mirror.
- Byte-level progress callback пока не интегрирован в `terminal-dialogue`; интерфейс показывает typed Running/Completed/Error state без ручного renderer output.
- Модель остаётся optional для lexical/code navigation, но её получение и retry больше не требуют ручной конфигурации.
