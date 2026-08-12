# B1 local vector comparison

Дата: `12.08.2026 17:20`. Exact candidate base: `d9bddda5a3c18af4dbe5d83e24c0a52d11f2a6ce`.

## Scope and privacy

Comparison uses only owner-approved read-only fixture roots `tests/fixtures/dt3/document-root` and `tests/fixtures/dt2`, logical root `document-representative-b1-fixture`. It does not replay the redacted A1 918-file runtime root; arbitrary representative-root acceptance remains E2. Model artifacts were provisioned before any fixture text, are isolated in ignored `.cfknowledge/dt3-b1-pv14/models`, and are identified by upstream revisions and SHA-256: BGE-M3 `5617a9f...` / `1eebfb28493f...` external ONNX data; E5 `614241f...` / `ca456c06...` ONNX; Qwen `97b0c614...` / `0437e45c...` safetensors. TEI image digest is `ad950d30878eceb72aaf32024d26fa2b1d04a75304fa0b4776b49aa1941fea07`.

## Results

| profile | result | causal evidence |
|---|---|---|
| multilingual-E5-small | accepted direction | Ten isolated local CPU runs returned finite 384-dim vectors; expected architecture fixture was rank 1 in every run; deterministic rank sequence was `0,1,2`; elapsed ms: 2152, 2095, 2101, 2091, 2092, 2126, 2118, 2141, 2077, 2079. |
| BGE-M3 | unavailable for this frozen adapter/export pair | FastEmbed `Bgem3Embedding::try_new_from_path` rejects the approved BAAI ONNX export: `expects the model to return 3 outputs ... got 2`; no vector was accepted. |
| Qwen3-Embedding-0.6B | unavailable | TEI GPU/ORT start fails because the approved local Qwen cache has no `/data/onnx/model.onnx`; TEI selected CPU fallback. Docker Desktop reported `80/tcp:[]` despite loopback publish on the required `--internal` no-egress network, and both host loopback and container address probes timed out. No corpus-bearing Qwen request was counted. |

E5 is the only eligible winner: quality first, with BGE and Qwen failing before valid-vector gates. The five first runs and five following runs are the required cold/warm sequence; all ten meet required-hit, must-not and 5/5 deterministic-order evidence for this fixture query. This spike does not implement B2 lifecycle or alter public contracts.
