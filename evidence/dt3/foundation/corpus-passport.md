# A1 Corpus passport

Дата: `12.08.2026 12:25`. Runtime roots are parameters, never committed. Logical roots are `document-representative` (DT2 readback: 918 Markdown, 30 TSV, 9 Python) and `code-fastsearch` (this repository's Rust source contour). Identity is SHA-256 over an inventory of relative locator, byte count and per-file SHA-256; no absolute path or source text is serialized.

Accepted support: Markdown/TSV document contour, Rust and Python structural contour only. All other extensions are `UNSUPPORTED_UNVERIFIED`. `.cfmap.md` is classified once as a map source, never ordinary Markdown. Raw corpus, credentials and external egress are excluded.

## Legacy transition — ACCEPTED

DT2 single-root identifiers migrate through a mandatory rebuild into `named-root-v1`: `logical_root_id + normalized '/' relative_locator + selector_kind + selector_value`, hashed/encoded by the A2 `StableId` constructor. Old persisted records are not silently interpreted as named-root IDs: opening legacy state is `Stale`, then rebuild is required. Same relative path in distinct logical roots is distinct; duplicate complete source keys are a typed failure. This preserves SQLite authority and makes source/projection provenance explicit.
