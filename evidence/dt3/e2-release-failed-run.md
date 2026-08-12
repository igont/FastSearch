# Диагностический release-прогон E2 R4

Измеренная product revision: `633eaa58c3cb73f793ef3436bb45332f51a42426`.
SHA-256 бинарника: `0e3b712719105adaf4581e2c70f3bf6a7ddc3a784efb072668fddec631df904b`.

Первый authoritative прогон на точных document/`src` inputs завершил все 20 процессов с exit code `0`, однако runner ошибочно дал release verdict `FAIL`: warm samples 3 и 4 заняли `575.127 ms` и `540.506 ms`, а runner применил к new-process reopen+query бюджет чистого in-process warm query `<=500 ms`. Остальные warm samples: `270.126`, `353.159`, `261.237 ms`. Cold max `530.512 ms`; service ratio `1.434960`; non-vector peak `43,401,216` bytes; E5 peak `1,521,287,168` bytes. Все gates кроме ошибочно классифицированного `warm_max_ms` были `true`.

Warm-команда runner — отдельный CLI-процесс `fastsearch search ... balanced Navigation`: она включает запуск процесса и открытие существующих SQLite/Tantivy projections, но не rebuild и не E5 model load. Принятый A1 PV-28 контракт классифицирует её как new-process startup+store reopen+query с бюджетом `<=750 ms`; все пять сырых samples первого прогона проходят этот бюджет. Чистый in-process warm query имеет отдельный бюджет `<=500 ms` и этим runner не измеряется. Повторный полный прогон тем же бинарником и семантикой дал warm max `282.027 ms`; обе серии и исходный ошибочный verdict сохранены, samples не отбрасывались.
