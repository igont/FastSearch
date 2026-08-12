# B2 — immutable model identity boundary

Дата: `12.08.2026 23:14`. Exact base: `054055f1f14c6dbe57905211f315d63208feb314`.

## Причинный RED

На exact base path-based цепочка была непосредственно подтверждена чтением и существующим lifecycle oracle: `model_manifest(root)` завершал hash, затем `embed(root, ...)` повторно открывал те же pathname; `search` также отпускал state lock до inference и без финальной revalidation возвращал старый snapshot как `Current`. Новый deterministic barrier-test до product-изменения дополнительно завершился `E0425` из-за отсутствия verify→load seam. Этот compile RED фиксирует отсутствие управляемого barrier, а причинный behavioral control — исходная двойная pathname-open цепочка и публичный stale-search путь.

## GREEN

- Полный B1 E5 manifest и фактически переданные FastEmbed байты теперь формируются из одного snapshot.
- На Windows каждый файл и каталог snapshot удерживается handle без `FILE_SHARE_WRITE` и `FILE_SHARE_DELETE` до завершения provider inference; reparse point отклоняется до чтения.
- Exact B1 allowlist ограничивает snapshot 43 файлами, 4 дочерними каталогами и фиксированными размерами до allocation/read; unexpected entry отклоняется до чтения.
- После no-follow open размер и regular-file type перепроверяются на acquired handle; чтение использует exact-size `read_exact` и однобайтный EOF probe вместо неограниченного `read_to_end`. Executable vulnerable control выполняет исходный порядок pathname-size check → ordinary-file swap → unrestricted second open/read и наблюдает лишние `1,048,576 B`. Тот же disposable oversized acquired handle передаётся production helper и получает `opened model file size mismatch`; удаление handle-size/read_exact/EOF логики ломает oracle.
- Barrier работает только на disposable exact copy. После verification он одновременно пытается перезаписать `onnx/config.json`, заменить внутренний `onnx` и создать junction на внешний sentinel; все операции отклонены, публичные apply/search возвращают Current/hit/provenance только для pinned bytes. Pre-existing junction control даёт typed error, no Current, zero hits/no provenance. Concurrent search/reconfigure gate доказывает линейризацию. Итог: `1 passed`, `71.26 s`.
- Полный stationary E5 lifecycle с add/change/delete-equivalent replacement, reopen, rebuild, manifest mutation, typed degradation и recovery повторно запущен после R5: `2 passed`, `339.96 s` тестового времени.
- Cleanup/readback: disposable copy удаляется RAII; внешний sentinel неизменен; canonical cache не атакуется и повторно подтвердил B1 manifest root `63A0FA9A...B194D0E` в stationary lifecycle.

## Широкие gates exact candidate

- `cargo fmt --check` — PASS.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` — PASS.
- `cargo test --workspace --all-targets --locked --no-fail-fast` — PASS; ignored real-E5 gates запущены отдельно выше.

Residual portability: строгий OS-denial mutation/replacement доказан на Windows. На не-Windows adapter использует immutable loaded-byte snapshot и отклоняет symbolic links, но Windows junction gate к другой платформе неприменим.
