# Safe service and evidence contract

Before a write, canonical service path must differ from every input root and must not be an ancestor. A descendant is allowed only exactly as `<DocumentRoot>/.cfknowledge/dt3-a1-<run-id>`. Any reparse point on its resolved path rejects the run. The runner writes only its exact marker/run JSON and readbacks both; cleanup is allowed only when both marker value and schema readback match the run ID.

Committed schema `dt3-a1-redacted-v1` permits logical root IDs, relative locators/selectors, ranks, hashes, timings and error classes. It forbids absolute paths, source text/snippets and secret-like keys/values. Negative runner cases cover equality, ancestor and wrong descendant; the redaction predicate rejects path/secret/content tokens before persistence.
