# Черновик интеграционного отчёта DT3 / E2

E2 связывает принятые document/vector/map/symbol adapters через единственную production-композицию и E1 fusion. Пути document root, code root, service root и optional local E5 root являются параметрами CLI; машинные пути не входят в публичную identity.

Production boundary проверяет service root до первой записи: внутри source разрешён только точный `.cfknowledge/<run>`, отдельный изолированный root разрешён, существующие reparse points запрещены. Cleanup ограничен точным marker-owned run-каталогом, отказывается удалять неизвестное содержимое и не выполняет рекурсивное удаление.

Lifecycle-oracle включает перенос сырого DT2 SQLite state, отсутствие ошибочного `get` до rebuild, переход к Current после rebuild, отказ настроенного E5 после Current без потери authority/ложных hits и последующее восстановление провайдера. Authoritative release JSON фиксирует product revision, SHA-256 бинарника, toolchain, exit code и digest семантического вывода каждого из 20 процессов; рабочий путь содержит пробелы.

Структурная навигация сообщает только синтаксические Rust/Python declaration facts и явные `.cfmap.md` relations. Она не заявляет compiler-resolved definition, references, type inference или call graph.

## Остаточные границы

- Исторический redacted A1 corpus из 918 файлов не имеет восстановимого machine path; release acceptance выполняется на согласованных deterministic fixture roots и текущем Rust source contour без ложного заявления о старом corpus.
- MCP/agent transport остаётся scope DT4. E2 предоставляет CLI/application boundary, но не добавляет transport.
- Qwen3-Embedding-0.6B остаётся в будущем сравнении моделей; production E2 использует принятый local multilingual-E5 contour.
- Governance/TDR controlled fields не изменялись. Для последующего writer/review/approval передаются evidence exact candidate, release JSON, quality 24/24 x 5 и master verdict.
