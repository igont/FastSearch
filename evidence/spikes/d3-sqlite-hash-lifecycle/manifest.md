# D3 evidence manifest — SQLite content-hash lifecycle spike

- Task / plan / leaf: `FASTSEARCH-DT1` / `PV-2` / `D3`.
- Exact A revision: `01e557899e5be846bd59f233edf81270dda89da0`.
- Accepted D2 / D3 base: `7e4893e509f6fba1233f650898fa3058289c4f84`.
- Scope: one synthetic stable identity and three UTF-8 payload/hash pairs only; no TDR or
  production corpus was read, and root source/manifests/locks were not changed.

## Reproduction

From the repository root execute:

```powershell
powershell -ExecutionPolicy Bypass -File spikes/d3-sqlite-hash-lifecycle/run.ps1 -Mode red
powershell -ExecutionPolicy Bypass -File spikes/d3-sqlite-hash-lifecycle/run.ps1 -Mode green
```

RED must exit `101`; GREEN must exit `0`. Each runner invocation creates and deletes its database
only below ignored `target/d3-sqlite-hash-lifecycle-evidence/`. `observations.md` records the
commands, fixed package version, output and cleanup result; replay cannot change tracked evidence.

## Contract classification

| Evidence | Affected contract/path | Category | TODO / backflow | A revision | Superseded / rerun | Verdict |
|---|---|---|---|---|---|---|
| `observations.md` RED | `spikes/d3-sqlite-hash-lifecycle/src/main.rs`: schema without unique stable identity | `adapter-only/internal` | `TODO[D3-SQLITE-LIFECYCLE-TRANSACTION-CLEANUP]`: future state adapter must enforce stable-identity uniqueness, compare content hashes inside its own transition boundary, and define its schema/transaction/cleanup policy without changing A stable-ID/hash/state-store contract. | `01e557899e5be846bd59f233edf81270dda89da0` | `ACCEPTED`; rerun only if A revision changes | causal RED: duplicate state is possible without identity uniqueness |
| `observations.md` GREEN | same isolated SQLite lifecycle boundary | `adapter-only/internal` | same exact TODO; no A2 backflow and no replan | `01e557899e5be846bd59f233edf81270dda89da0` | `ACCEPTED`; rerun only if A revision changes | `подходит с ограничениями` |

## Observations and limits

- A public `StableId` and three public `ContentHash` values cross the local SQLite boundary
  unchanged.
- One identity is added once; an equal hash is unchanged; each of two changed hashes updates one
  row; final delete removes that identity.
- The runner proves temporary database cleanup after both RED and GREEN.
- This spike does not choose a production state-store API, schema, migration, concurrency,
  transaction isolation, crash recovery or performance policy.

## G-08.D3 handoff

Decision: `подходит с ограничениями`.

Full category: `adapter-only/internal`. Backflow is limited to
`TODO[D3-SQLITE-LIFECYCLE-TRANSACTION-CLEANUP]`; no public stable-ID/hash/state contract mismatch
was observed. Root must evaluate `G-08.D3`, then compare all accepted D1–D3 records in
`G-08.FINAL`; B remains closed until that final PASS. If A receives a new accepted revision, all
D evidence becomes `SUPERSEDED` and D restarts at D1.
