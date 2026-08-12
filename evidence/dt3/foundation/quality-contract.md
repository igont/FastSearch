# A1 Quality contract

Sections are immutable for this tree: document/vector (`A1/B1`), map (`C1/C2`), symbol (`D1/D2`), fusion (`E1`). A gated section is `NOT_APPLICABLE_UNTIL_<gate>` and is excluded, never counted as zero.

Every evaluated row uses the logical selector from `queries.tsv`. Required-hit rate = rows with an expected selector at rank `<= required_rank_max` / evaluated rows. Must-not violations = returned forbidden selectors / evaluated rows. Exact rows require rank 1 in 100%; must-not violations = 0; deterministic dedupe/order = 5/5 repeated identical selector sequences. Lexical/paraphrase require 100% required-hit within their declared rank. Map (`NOT_APPLICABLE_UNTIL_C1_C2`) and symbol (`NOT_APPLICABLE_UNTIL_D1_D2`) use the same formulas and require 100% required-hit, 0 must-not, and 5/5 repetition once admitted. Fusion (`NOT_APPLICABLE_UNTIL_E1`) requires all admitted channel selectors preserved, 0 duplicates, and lexicographic source-key tie order in 5/5 repeats.
