# A1 Performance contract

Release-mode benchmark uses five cold and five warm samples on the parameterized document root. Acceptance budgets: rebuild median <= 12,000 ms, rebuild max <= 18,000 ms; unchanged update median <= 5,000 ms, max <= 8,000 ms; warm exact query median <= 250 ms, max <= 500 ms; service bytes <= 2.0 x source bytes; peak working set <= 1,024 MiB. DT2/A1 regression ratios: rebuild <= 1.40, update <= 1.40, warm query <= 1.50, service bytes <= 1.50, peak memory <= 1.50. A sample outside a budget is a gate failure, not an averaged-away outlier.
