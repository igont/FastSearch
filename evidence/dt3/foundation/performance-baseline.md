# A1 Release performance baseline

Revision: `135572ff45321005ca75c6bc3624e1525f99bce7`; binary: `target/release/fastsearch.exe` from `cargo build --release --locked`; Windows/PowerShell; network disabled. Parameterized runtime root was the owner-provided logical `document-representative`; service was the fresh exact run zone `document-representative/.cfknowledge/dt3-a1-perf-01`. The run zone contained 146,977,670 bytes; source inventory 273,268,956 bytes. No locator, source text or absolute path is committed.

Command template: `fastsearch index rebuild <DocumentRoot> <ServiceRoot>`, then `index update`, then `search <DocumentRoot> <ServiceRoot> balanced "document"`. Each cold sample used a new exact `.cfknowledge/dt3-a1-perf-cold-<n>` run zone; each following update/query is the corresponding warm sample. Stopwatch milliseconds include process startup. Peak working set was captured by polling the child process every 5 ms during an independent five-run repeat.

| run | rebuild ms | update ms | query ms |
|---:|---:|---:|---:|
| 1 | 1556 | 1809 | 14 |
| 2 | 1528 | 1771 | 17 |
| 3 | 1545 | 1663 | 15 |
| 4 | 1546 | 1664 | 14 |
| 5 | 1462 | 1696 | 15 |
| median | 1545 | 1696 | 15 |
| max | 1556 | 1809 | 17 |

Peak working-set samples (rebuild/update/query, bytes) were respectively: `148955136/103763968/5799936`, `145330176/103624704/8224768`, `145801216/103866368/4251648`, `145440768/103493632/6844416`, `148332544/103944192/6963200`; overall maximum is `148955136` bytes (142.05 MiB). All measured values meet A1 absolute budgets. Service/source ratio is `0.394` and meets the 2.0 limit.
