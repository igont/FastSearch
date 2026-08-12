# D1 Tree-sitter identity spike

Дата: `12.08.2026 12:25`.

## Принятое ограниченное решение

Фиксированный A2 envelope достаточен для D2: `tree-sitter = 0.25.10`,
`tree-sitter-rust = 0.24.0`, `tree-sitter-python = 0.25.0`. Все три crate
декларируют MIT; версии уже зафиксированы в `Cargo.lock`.

Для Rust применяются captures `(function_item) @declaration` и
`(struct_item) @declaration`; для Python — `(function_definition) @declaration`
и `(class_definition) @declaration`. Для каждого capture D2 должен читать
только `name` и structural node kind, не запрашивая compiler definition,
reference или type information.

## Идентичность и отказ

Прототипированная identity: `structural-v1:{logical_root}:{relative_locator}:{language}:{kind}:{name}:{start_byte}`.
Она не содержит абсолютного пути или Tree-sitter node ID. `start_byte` различает
одноимённые декларации одного файла; rename меняет canonical relative locator.

До парсинга лимит — 64 KiB исходного текста, затем не более 512 посещённых
узлов. Неподдерживаемое расширение, превышенный лимит, ошибка синтаксиса,
неподходящий query или отсутствующее UTF-8 имя завершают обработку без
публикации частичного набора символов.

## Доказательство и граница D2

`cargo test --locked --test d1_tree_sitter_identity` проверяет Rust/Python
fixtures, повторный разбор, duplicate names, Unicode locator, rename, syntax
error, unsupported extension, byte/node limits. `cargo test --locked` и
`cargo fmt --check` прошли на candidate.

D2 должен перенести эту стратегию в `src/adapters/symbols/**`, подключив её к
принятому named-root/source lifecycle. Производственный runtime, retrieval,
rename/delete/reopen/rebuild lifecycle и public API не реализованы этим spike.
