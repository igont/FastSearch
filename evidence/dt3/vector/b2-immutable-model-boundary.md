# B2 — immutable model identity boundary

Дата: `12.08.2026 23:14`. Exact base: `054055f1f14c6dbe57905211f315d63208feb314`.

## Причинный RED

Новый deterministic barrier-test `verified_model_denies_mutation_and_replacement_until_provider_finishes` был добавлен до product-изменения. На exact base сборка завершилась `E0425`: отсутствовала `install_verify_load_hook`, то есть код не имел проверяемой границы между manifest verification и provider load.

## GREEN

- Полный B1 E5 manifest и фактически переданные FastEmbed байты теперь формируются из одного snapshot.
- На Windows каждый файл и каталог snapshot удерживается handle без `FILE_SHARE_WRITE` и `FILE_SHARE_DELETE` до завершения provider inference; reparse point отклоняется до чтения.
- Barrier после verification одновременно попытался перезаписать `onnx/config.json` и заменить корень модели. Обе операции были отклонены ОС, E5 inference завершился на pinned verified bytes: `1 passed`, `21.79 s`.
- Полный stationary E5 lifecycle с add/change/delete-equivalent replacement, reopen, rebuild, manifest mutation, typed degradation и recovery: `2 passed`, `386.40 s` тестового времени.
- Cache readback после barrier: полный lifecycle повторно подтвердил B1 manifest root `63A0FA9A...B194D0E`; внешняя модель не изменена, race replacement не создан.

## Широкие gates exact candidate

- `cargo fmt --check` — PASS.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — PASS.
- `cargo test --workspace --all-targets --locked --no-fail-fast` — PASS; ignored real-E5 gates запущены отдельно выше.

Residual portability: строгий OS-denial mutation/replacement доказан на Windows. На не-Windows adapter использует immutable loaded-byte snapshot и отклоняет symbolic links, но Windows junction gate к другой платформе неприменим.
