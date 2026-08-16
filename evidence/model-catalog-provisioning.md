# Selectable embedding catalog — provisioning evidence

Дата: 2026-08-16.

## Проверенный каталог

| Stable ID | Source | Revision | Runtime | Dimension |
|---|---|---|---|---:|
| `multilingual-e5-small` | `intfloat/multilingual-e5-small` | `614241f622f53c4eeff9890bdc4f31cfecc418b3` | ONNX | 384 |
| `multilingual-e5-base` | `intfloat/multilingual-e5-base` | `d128750597153bb5987e10b1c3493a34e5a4502a` | ONNX | 768 |
| `multilingual-e5-large` | `Qdrant/multilingual-e5-large-onnx` | `66076b8dc6e367337e3e90e6fb309fb0f3addaf6` | ONNX | 1024 |
| `qwen3-embedding-0.6b` | `Qwen/Qwen3-Embedding-0.6B` | `97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3` | Candle/F32/CPU | 1024 |
| `nomic-embed-text-v2-moe` | `nomic-ai/nomic-embed-text-v2-moe` | `1066b6599d099fbb93dfcb64f9c37a7c9e503e85` | Candle/F32/CPU | 768 |

## Real-network acceptance

Команда:

```text
cargo test --test model_catalog_pipeline --locked -- --ignored --nocapture
```

Результат: `1 passed; 0 failed`, 1516.72 s. Тест последовательно вызвал product provisioning для всех пяти IDs. Для каждой модели были доказаны полная загрузка обязательных assets, открытие runtime, synthetic inference, finite vector и точная dimension. Workspace, source roots и indexes тест не создаёт.

Во время прогона Hugging Face CDN дважды переставал отдавать байты. Первоначальный transport требовал перезапуска процесса. После добавления product-owned timeout/retry/HTTP Range pipeline самостоятельно прервал нулевую попытку Qwen, продолжил `.download` и завершил Qwen и Nomic без ручного вмешательства. Частичная E5 Large загрузка также была успешно продолжена.

Отдельный `cargo test --test e5_auto_pipeline --locked -- --ignored --nocapture` подтвердил уже полный explicit lifecycle для default model: provisioning → явная индексация → vector search. Это не implicit indexing после установки.

## Граница доказанного

- Все пять runtime готовы к controlled corpus experiment, но только E5 Small пока имеет прежний exact immutable manifest qualification и остаётся default.
- Сравнение качества, latency и ресурсов выполняется на одном versioned corpus; `/experiment record <оценка>` сохраняет model/query/hits/latency/judgment.
- Проверен CPU fallback на Windows. GPU execution profile и Linux acceptance являются отдельными будущими gates.
- В интерактивном terminal-dialogue пока показывается typed stage, а не byte-level progress bar.
