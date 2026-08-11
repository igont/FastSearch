# D3 observed output

## Tool version

- `rusqlite = 0.40.2` with the `bundled` SQLite feature, fixed in the isolated D3 package
  `Cargo.toml` and `Cargo.lock`.

## Causal RED

```powershell
powershell -ExecutionPolicy Bypass -File spikes/d3-sqlite-hash-lifecycle/run.ps1 -Mode red
```

Outer exit is `101`. A table without stable-identity uniqueness accepts two rows for the same
synthetic identity; the oracle expected count `1` and observed `2`. The runner then removed the
temporary database under ignored `target/d3-sqlite-hash-lifecycle-evidence/`.

## GREEN

```powershell
powershell -ExecutionPolicy Bypass -File spikes/d3-sqlite-hash-lifecycle/run.ps1 -Mode green
```

Outer exit is `0`; runner output records `database_existed_before_cleanup=True` and
`database_removed=True`. Spike stdout is:

```text
D3 PASS: add=1; unchanged=1; changed=2; delete=1; stable_id_and_hash=preserved
```

The synthetic fixture has one `StableId` and three deterministic payload/hash pairs. Each stored
hash is checked after add, unchanged, and each change; the final delete leaves no row.
