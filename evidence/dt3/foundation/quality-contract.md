# A1 Quality contract

Sections are immutable for this tree: document/vector (`A1/B1`), map (`C1/C2`), symbol (`D1/D2`), fusion (`E1`). A gated section is `NOT_APPLICABLE_UNTIL_<gate>` and is excluded, never counted as zero.

Every evaluated row uses the logical selector from `queries.tsv`. Required-hit rate = rows with an expected selector at rank `<= required_rank_max` / evaluated rows. Must-not violations = returned forbidden selectors / evaluated rows. Exact rows require rank 1 in 100%; must-not violations = 0; deterministic dedupe/order = 5/5 repeated identical selector sequences. Lexical/paraphrase require 100% required-hit within their declared rank; map/symbol are admitted only after their producer gates. Fusion formula and threshold are `NOT_APPLICABLE_UNTIL_E1`.
