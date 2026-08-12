# Черновик интеграционного отчёта DT3 / E2

E2 связывает принятые document/vector/map/symbol adapters через единственную production-композицию и E1 fusion. Пути document root, code root, service root и optional local E5 root являются параметрами CLI; машинные пути не входят в публичную identity.

Production boundary проверяет service root до первой записи: внутри source разрешён только точный `.cfknowledge/<run>`, отдельный изолированный root разрешён, существующие reparse points запрещены. Cleanup ограничен точным marker-owned run-каталогом, отказывается удалять неизвестное содержимое и не выполняет рекурсивное удаление.

Lifecycle-oracle включает перенос сырого DT2 SQLite state, отсутствие ошибочного `get` до rebuild, переход к Current после rebuild, отказ настроенного E5 после Current без потери authority/ложных hits и последующее восстановление провайдера. Authoritative release JSON фиксирует измеренную product revision, отношение evidence-only candidate, SHA-256 бинарника, toolchain, exit code и digest семантического вывода каждого из 20 процессов; рабочий путь содержит пробелы. Входы привязаны к logical identities и SHA-256 inventories относительных locators: deterministic document fixture и точный текущий `src` contour из 21 файла.

Windows runtime создаёт отсутствующую цепочку service по одному компоненту и немедленно закрепляет каждый компонент no-follow handle до любой descendant-записи; `runs` закрепляется тем же способом до открытия SQLite/Tantivy. Handles service/runs и каждого exact run-child удерживаются без `FILE_SHARE_DELETE`; marker writes и cleanup выполняются при закреплённом child handle, а пустой каталог помечается на удаление через тот же handle. Coordinated bootstrap и exact-run rename/delete/junction races не получают path-based промежутка; внешние sentinels и отсутствие внешних DB/index подтверждаются тестами.

Release runner до первой записи требует попарно непересекающиеся document/code/work/runtime-document/E5 roots в обоих направлениях и отвергает junction в существующих ancestors. Caller-supplied runtime-document root не должен существовать и создаётся вне WorkRoot; старый inside-WorkRoot pattern отклоняется до записи. Source bytes вычисляются из замороженных runtime document/code inventories до запуска процессов; после прогона file count и manifest SHA-256 должны точно совпасть. Service ratio учитывает только явные service roots, а не scaled source copy или произвольные каталоги work root. Release runner измеряет отдельный new-process startup+SQLite/Tantivy reopen+query gate `<=750 ms`, а не чистый in-process warm query `<=500 ms`. Обе наблюдавшиеся серии сохранены; исходный ошибочный 500-ms verdict и сырые значения зафиксированы отдельным диагностическим evidence.

Структурная навигация сообщает только синтаксические Rust/Python declaration facts и явные `.cfmap.md` relations. Она не заявляет compiler-resolved definition, references, type inference или call graph.

## Остаточные границы

- Исторический redacted A1 corpus из 918 файлов не имеет восстановимого machine path; release acceptance выполняется на согласованных deterministic fixture roots и текущем Rust source contour без ложного заявления о старом corpus.
- MCP/agent transport остаётся scope DT4. E2 предоставляет CLI/application boundary, но не добавляет transport.
- Qwen3-Embedding-0.6B остаётся в будущем сравнении моделей; production E2 использует принятый local multilingual-E5 contour.
- Governance/TDR controlled fields не изменялись. Для последующего writer/review/approval передаются evidence exact candidate, release JSON, quality 24/24 x 5 и master verdict.
