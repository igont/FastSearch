# D2 structural-symbol producer contract

Дата: `12.08.2026 16:25`.

`SymbolSource` принимает только явно заданный `LogicalRootId` и replaceable code
root. Поддерживаются исключительно `.rs` и `.py`; результат — `CodeSymbol` с
metadata `fact_kind=structural source symbol`, без definition/reference/type
claim. Stable id строится штатным `named-root-v1` из logical root, normalized
relative locator и selector `language:kind:name:start_byte`.

Полный snapshot fail-closed: invalid UTF-8, syntax error, unsupported extension,
query/name failure, size 64 KiB (проверяется metadata до read), depth 16,
files 1024 или nodes 16 384 возвращают ошибку до выдачи частичного corpus. Symlink
не обходятся. `find_symbols` возвращает
case-insensitive title matches в порядке stable id.

`tests/d2_symbol_lifecycle.rs` проверяет independent named roots, deterministic
repeat, duplicate symbols, absence absolute path, search, SQLite
add/rename/delete/reopen и fail-closed syntax/unsupported/oversize cases. GREEN:
targeted 4/4, fmt, clippy all-targets и full
`cargo test --locked` прошли. D2 не меняет document source, public ports,
application composition или compiler semantics.

## PV-25 node-budget amendment

На exact base `cc91f63780357538b2e86e78adc8bee7fd76f510` старый лимит 512
отвергал 14 из 20 реальных Rust-файлов. Принятый лимит 16 384 — минимальная
степень двойки выше observed maximum 10 302; запас составляет 6 082 узла
(59,04%). Query, identity и остальные resource limits не менялись.

Exact `src` passport: 20 файлов, 50 451 узел, 331 declaration capture; максимум
`adapters/source/mod.rs` — 36 145 bytes, 10 302 nodes, 45 captures. Повторный
snapshot имеет те же identities и порядок. Dense fixture с 1 000 функциями
(6 002 nodes) проходит, fixture с 3 000 функциями (18 002 nodes, менее 64 KiB)
возвращает ошибку без snapshots. Отдельно повторены depth 17, files 1 025,
syntax, unsupported extension, oversize, Rust/Python и SQLite lifecycle gates.

## PV-26 integration robustness

Паспорт 20 файлов / 50 451 узел остаётся immutable evidence exact revision
`cc91f63780357538b2e86e78adc8bee7fd76f510`, а не вечным равенством для
расширяемого `src`. Executable current-src gate теперь дважды строит полный
sorted admitted inventory во время теста, требует непустой идентичный результат,
проверяет полное совпадение inventory со snapshot locators и ограничивает каждый
файл значением не более 16 384 nodes. Новый accepted Rust-файл не создаёт ложный
регресс, но превышение resource budget по-прежнему отклоняется.
