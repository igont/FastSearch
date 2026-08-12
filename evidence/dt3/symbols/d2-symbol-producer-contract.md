# D2 structural-symbol producer contract

Дата: `12.08.2026 16:25`.

`SymbolSource` принимает только явно заданный `LogicalRootId` и replaceable code
root. Поддерживаются исключительно `.rs` и `.py`; результат — `CodeSymbol` с
metadata `fact_kind=structural source symbol`, без definition/reference/type
claim. Stable id строится штатным `named-root-v1` из logical root, normalized
relative locator и selector `language:kind:name:start_byte`.

Полный snapshot fail-closed: invalid UTF-8, syntax error, unsupported extension,
query/name failure, size 64 KiB (проверяется metadata до read), depth 16,
files 1024 или nodes 512 возвращают ошибку до выдачи частичного corpus. Symlink
не обходятся. `find_symbols` возвращает
case-insensitive title matches в порядке stable id.

`tests/d2_symbol_lifecycle.rs` проверяет independent named roots, deterministic
repeat, duplicate symbols, absence absolute path, search, SQLite
add/rename/delete/reopen и fail-closed syntax/unsupported/oversize cases. GREEN:
targeted 4/4, fmt, clippy all-targets и full
`cargo test --locked` прошли. D2 не меняет document source, public ports,
application composition или compiler semantics.
