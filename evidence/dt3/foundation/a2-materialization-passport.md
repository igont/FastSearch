# A2 materialization decision map

| Boundary | Decision / owner |
|---|---|
| Named-root identity | A2 owns `src/domain/**`: `named-root-v1` source key and versioned `StableId`; mandatory rebuild of DT2 legacy state. |
| State/source lifecycle | A2 owns existing source/state/application migration seams; SQLite remains authority, projections rebuildable. |
| Shared modules | A2 owns `src/ports/mod.rs` and `src/adapters/mod.rs`; no optional vector/map/symbol crate is added in A2. |
| Existing exact crates | `tantivy = 0.26.1`, `rusqlite = 0.40.2`, `sha2 = 0.10.9`, `ignore = 0.4.33` remain frozen; future adapter dependency requires A backflow, not B/C/D local edit. |
| Code admission | A2 provides named-root/source-admission boundary; D owns structural parser implementation. |
| `.cfmap.md` | single `CodeMap` source class; C owns schema and lifecycle; never ordinary Markdown. |
| Provider | B owns approved implementation only after a later owner decision; A2 retains typed unavailable envelope. |

All unknown production model/provider/store choices are visible `UNVERIFIED` gaps, not defaults.
