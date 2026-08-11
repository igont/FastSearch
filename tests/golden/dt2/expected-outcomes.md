# DT2 expected outcomes

| Scenario | Fixture/query | Expected observable result | Owner |
|---|---|---|---|
| competing current record | `guide-current` / `документальный поиск` | Russian FTS returns `guide-current` before equal-score `guide-design` in `Current` | D |
| competing design record | `guide-design` / `документальный поиск` | `Design` may prefer `guide-design`; equal final score uses deterministic StableId order | D |
| technical exact ID | `registry-2433` / `2433` | Exact is returned before lexical candidates | D |
| Russian FTS | `guide-current` / `русская фраза` | lexical hit is returned through `Lexical` channel | D |
| excluded source | `excluded.tmp` | excluded file creates no record and no search result | B |
| add | `guide-current` | update reports add and records become current | C |
| unchanged | unchanged source bytes | update reports unchanged with no projection mutation | C |
| change | changed source bytes | update reports change; stale projection is not reported current | C |
| delete | removed `guide-design` | update reports delete; removed record is absent after reopen | C |
| rebuild | persisted corpus | rebuild yields the same observable records and query results after process reopen | E |
| status | lifecycle mutation | status distinguishes current, stale and degraded without a false Real claim | E |
