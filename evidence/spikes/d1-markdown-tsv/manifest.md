# D1 evidence manifest — Markdown / TSV canonical record spike

- Task / plan / leaf: `FASTSEARCH-DT1` / `PV-2` / `D1`.
- Exact A revision: `01e557899e5be846bd59f233edf81270dda89da0`.
- Scope: two UTF-8 synthetic fixtures only; no TDR or production corpus was read.

## Reproduction

From the repository root execute:

```powershell
powershell -ExecutionPolicy Bypass -File spikes/d1-markdown-tsv/run.ps1 -Mode red
powershell -ExecutionPolicy Bypass -File spikes/d1-markdown-tsv/run.ps1 -Mode green
```

`red` deliberately supplies one shared identity and one shared content hash to two unlike
fixtures. It must fail `assert_ne!`; `red-command.txt` records exit `101` and
`red-observation.md` records the causal assertion. `green` supplies deterministic, distinct
stable identifiers and SHA-256 values of the two fixture files; it must exit `0`.

## Contract classification

| Evidence | Affected contract/path | Category | TODO / backflow | A revision | Superseded / rerun | Verdict |
|---|---|---|---|---|---|---|
| `red-command.txt`, `red-observation.md` | candidate record translation in `spikes/d1-markdown-tsv/main.rs`; public check through `fastsearch::domain::{CanonicalRecord, StableId, SourceLocator, ContentHash}` | `adapter-only/internal` | `TODO[D1-ADAPTER-NORMALIZATION]`: the future source adapter must parse real Markdown/TSV and derive accepted stable IDs, locators, metadata and content hashes before calling `CanonicalRecord::new`; this TODO does not alter the public A contract. | `01e557899e5be846bd59f233edf81270dda89da0` | `ACCEPTED`; rerun only if A revision changes | causal RED: shared identity is rejected by the oracle |
| `green-command.txt`, `green-run.stdout.txt` | same public record boundary; readonly source `src/domain/record.rs` | `adapter-only/internal` | same exact future TODO; no A2 backflow and no replan | `01e557899e5be846bd59f233edf81270dda89da0` | `ACCEPTED`; rerun only if A revision changes | `подходит с ограничениями` |

## Observations

- `CanonicalRecord` accepts and preserves separate stable IDs, record kinds, source paths and
  selectors for one Markdown H1 section and one TSV data row.
- Searchable content, title, format/owner/status metadata, explicit empty relations and distinct
  SHA-256 values survive the public boundary.
- The spike does not select a parser, retrieval behavior, ranking algorithm or hash lifecycle.

## G-08.D1 handoff

Decision: `подходит с ограничениями`.

Full category: `adapter-only/internal`. Exact backflow is limited to
`TODO[D1-ADAPTER-NORMALIZATION]`; public record/identity/locator/content/hash contracts remain
unchanged. Root may evaluate `G-08.D1`; only its PASS authorizes D2. If A receives a new accepted
revision, both rows above become `SUPERSEDED` and D restarts at D1.
