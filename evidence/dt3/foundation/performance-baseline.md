# A1 Release performance baseline

Revision: `135572ff45321005ca75c6bc3624e1525f99bce7`; binary: `target/release/fastsearch.exe` from `cargo build --release --locked`; Windows/PowerShell; network disabled. Parameterized runtime root was the owner-provided logical `document-representative`; service was the fresh exact run zone `document-representative/.cfknowledge/dt3-a1-perf-01`. The run zone contained 146,977,670 bytes; source inventory 273,268,956 bytes. No locator, source text or absolute path is committed.

Command template: `fastsearch index rebuild <DocumentRoot> <ServiceRoot>`, then `index update`, then `search <DocumentRoot> <ServiceRoot> balanced "документальный поиск"`. Stopwatch milliseconds include process startup. The first rebuild is cold; remaining samples are warm-cache observations.

| run | rebuild ms | update ms | query ms |
|---:|---:|---:|---:|
| 1 | 10927 | 1857 | 13 |
| 2 | 2594 | 2152 | 12 |
| 3 | 3135 | 2847 | 16 |
| 4 | 3521 | 1926 | 9 |
| 5 | 2224 | 1772 | 8 |
| median | 3135 | 1926 | 12 |
| max | 10927 | 2847 | 16 |

All measured values meet A1 absolute budgets. Peak working set was not measured: `UNVERIFIED`; therefore the final performance gate remains conditional on a repeatable peak-memory capture before A2 PASS. Service/source ratio is `0.538` and meets the 2.0 limit.
