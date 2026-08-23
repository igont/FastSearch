# Снимок синхронизации документации

- Техническая редакция: `c795c7897ada2de95225132d24dccdb338820699`.
- `documentation_candidate_hash`: `88ae97db152919fd74e711af8dacfee9d9620964c871adf823916cdf2c8fc589`.
- Размер канонического манифеста: 888 байт.
- Состояние упаковки: переходный снимок для `dtree 0.3.1`; версия бинарника ещё не предоставляет команду сохранения документационного артефакта.

Канонические байты манифеста находятся в следующей строке без внешних пробелов и завершающего перевода строки:

```json
{"schema":"dtree-documentation-snapshot/v1","technical_integration_revision":"c795c7897ada2de95225132d24dccdb338820699","entries":[{"path":"Obsidian/Docs/TDR/TDR-FS-2.4 Automatic model provisioning.md","status":"present","size_bytes":21353,"sha256":"44dfecfac34bb8ae0871b411f086b5066ff3bd94a1adf59d74fb011069e7f40d"},{"path":"README.md","status":"present","size_bytes":29486,"sha256":"c576b6b03e92d80c3d15a42c0c945e322531c535695a1fddc8b4b8f98615999d"},{"path":"ROADMAP.md","status":"present","size_bytes":38822,"sha256":"25912c18be8f399b22c9faa955ef7aba5f5c8bf5b7f9f427ff567ce21e736f55"},{"path":"evidence/dt4/documentation.sync-input.md","status":"present","size_bytes":2559,"sha256":"1a65b2f9cd473b90bc6f8b5ae4206580b199e5eb46604bedf1db7258568bad97"}],"excluded_classes":["ARTIFACT_PROJECTION","EVENT_PROJECTION","REVIEW_INPUT","STATE_PROJECTION","RELEASE_RECEIPT","SNAPSHOT_MANIFEST"]}
```

Снимок содержит четыре авторских документа. Сам этот файл, рецензия, выпускная квитанция и отчёт синхронизации исключены из хэша.
