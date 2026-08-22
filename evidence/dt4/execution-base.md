# Свидетельство G-EXECUTION-BASE

- Дата проверки: `22.08.2026`.
- Плановая база: `aa7a7d45033098b1690691c47b4dadbd5f7b084e`.
- `evidence/dt4/plan-baseline.sha256`: все девять нормативных файлов совпали с зафиксированными SHA-256.
- Git-происхождение: исполнительная редакция является потомком плановой базы; рабочее дерево после фиксации чисто.
- Исходные проверки: `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --all-targets --locked --no-fail-fast` и `cargo build --release --locked` завершились успешно.

## Регистрация корпуса

FastSearch не входит в зарегистрированный локальный корпус. В `%LOCALAPPDATA%/FastSearch/catalog.json` на момент проверки присутствовали только области `CF_Spring_Server/Obsidian`, `FastGraph` и `C:/Obsidian`; пути `D:/Igor/Programming/FastSearch` и его предков отсутствовали. В корне репозитория также отсутствует `.fastsearch/workspace.toml`. Поэтому A1 не требует обновления индекса после публикации документов.
