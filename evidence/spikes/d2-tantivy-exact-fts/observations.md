# D2 observed output

## Tool version

- `tantivy = 0.26.1`, fixed in `spikes/d2-tantivy-exact-fts/Cargo.toml` and its `Cargo.lock`.

## Causal RED

```powershell
powershell -ExecutionPolicy Bypass -File spikes/d2-tantivy-exact-fts/run.ps1 -Mode red
```

Outer exit is `101`. With one combined `TEXT` field a query for `ZX42` returns both:

```text
markdown:synthetic/exact-document.md#ZX42
markdown:synthetic/russian-phrase.md#RUS-001
```

The oracle expects only the first stable ID. Therefore the unseparated configuration cannot
prove an exact technical-identifier intent.

## GREEN

```powershell
powershell -ExecutionPolicy Bypass -File spikes/d2-tantivy-exact-fts/run.ps1 -Mode green
```

Outer exit is `0` and stdout is:

```text
D2 PASS: exact_id=markdown:synthetic/exact-document.md#ZX42; russian_phrase=markdown:synthetic/russian-phrase.md#RUS-001; no_hit=explicit
```

The spike separately verifies an empty phrase and a nonmatching Russian phrase as explicit
empty hit lists.
