# Повторный снимок синхронизации документации

- Техническая редакция: `c795c7897ada2de95225132d24dccdb338820699`.
- `documentation_candidate_hash`: `02258b5579ac5c0376535c894ff4dc4e229d2353d8233bca6b9b69b84026d261`.
- Предыдущий хэш: `88ae97db152919fd74e711af8dacfee9d9620964c871adf823916cdf2c8fc589`.
- Размер канонического манифеста: 1048 байт.
- Состояние упаковки: переходный снимок для `dtree 0.3.1`; версия бинарника ещё не предоставляет команду сохранения документационного артефакта.

Канонические байты манифеста находятся в следующей строке без внешних пробелов и завершающего перевода строки:

```json
{"schema":"dtree-documentation-snapshot/v1","technical_integration_revision":"c795c7897ada2de95225132d24dccdb338820699","entries":[{"path":"Obsidian/Docs/Roadmap/00 Roadmap.md","status":"present","size_bytes":2461,"sha256":"465d909b28a49432d4016af242c7dfa6041a0842236887d82f81313d0de54813"},{"path":"Obsidian/Docs/TDR/TDR-FS-2.4 Automatic model provisioning.md","status":"present","size_bytes":21353,"sha256":"44dfecfac34bb8ae0871b411f086b5066ff3bd94a1adf59d74fb011069e7f40d"},{"path":"README.md","status":"present","size_bytes":29781,"sha256":"b6b999601b17ca710728adb70d58dee3da029a46348aceafe3c8aa4c1309bb5e"},{"path":"ROADMAP.md","status":"present","size_bytes":38854,"sha256":"6dc13275bd6270e37b3d96b06d173d7a66ad16dcc1ba99a65aff01a157d32a44"},{"path":"evidence/dt4/documentation.sync-input.md","status":"present","size_bytes":2934,"sha256":"a1f5c071d71fa7c0ccad8c4f727977a91d748717b3877e6800f3bad586cef9df"}],"excluded_classes":["ARTIFACT_PROJECTION","EVENT_PROJECTION","REVIEW_INPUT","STATE_PROJECTION","RELEASE_RECEIPT","SNAPSHOT_MANIFEST"]}
```

Снимок содержит пять авторских документов. Сам этот файл, первый снимок, рецензии, выпускная квитанция и отчёт синхронизации исключены из хэша.
