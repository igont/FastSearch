# D2 evidence manifest — Tantivy exact identifier and Russian FTS spike

- Task / plan / leaf: `FASTSEARCH-DT1` / `PV-2` / `D2`.
- Exact A revision: `01e557899e5be846bd59f233edf81270dda89da0`.
- Accepted D1 / D2 base: `e05fd7b86db19d21f066f143682b17704af1d1cf`.
- Scope: two synthetic UTF-8 documents only; no TDR or production corpus was read, and the root
  Cargo manifest and root Cargo lock were not changed.

## Reproduction

Run from the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File spikes/d2-tantivy-exact-fts/run.ps1 -Mode red
powershell -ExecutionPolicy Bypass -File spikes/d2-tantivy-exact-fts/run.ps1 -Mode green
```

The RED command must exit `101`; GREEN must exit `0`. Both commands write replay artifacts only
under ignored `target/d2-tantivy-exact-fts-evidence/`; a replay cannot modify versioned evidence
or the frozen candidate. `observations.md` records the commands, locked Tantivy version and
observed outputs.

## Contract classification

| Evidence | Affected contract/path | Category | TODO / backflow | A revision | Superseded / rerun | Verdict |
|---|---|---|---|---|---|---|
| `observations.md` RED | `spikes/d2-tantivy-exact-fts/src/main.rs`: combined `TEXT` field and query parser experiment | `adapter-only/internal` | `TODO[D2-TANTIVY-SCHEMA-ANALYZER]`: future lexical adapter must preserve separate exact-identifier and Russian-text fields, and make analyzer/normalization choices explicit before index construction; it must not change A query/result/error/status semantics. | `01e557899e5be846bd59f233edf81270dda89da0` | `ACCEPTED`; rerun only if A revision changes | causal RED: one field returns both stable IDs and does not establish exact intent |
| `observations.md` GREEN | same local Tantivy schema/query boundary | `adapter-only/internal` | same exact TODO; no A2 backflow and no replan | `01e557899e5be846bd59f233edf81270dda89da0` | `ACCEPTED`; rerun only if A revision changes | `подходит с ограничениями` |

## Observations and limits

- `STRING`/`TermQuery` exact lookup returns only
  `markdown:synthetic/exact-document.md#ZX42`.
- A quoted Russian phrase uses only `russian_text` and returns only
  `markdown:synthetic/russian-phrase.md#RUS-001`.
- Empty and nonmatching phrase inputs return explicit no-hit results, not a panic.
- This does not choose production scoring, analyzer/morphology behavior, performance limits,
  production schema or a public retrieval contract.

## G-08.D2 handoff

Decision: `подходит с ограничениями`.

Full category: `adapter-only/internal`. Backflow is limited to
`TODO[D2-TANTIVY-SCHEMA-ANALYZER]`; no public searchable representation, query/result/error/status
contract change was observed. Root must evaluate `G-08.D2`; only its PASS may authorize D3. If A
receives a new accepted revision, D1/D2 evidence becomes `SUPERSEDED` and D restarts at D1.
