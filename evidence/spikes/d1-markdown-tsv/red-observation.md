# Causal RED observation

`D1_FORCE_SHARED_VALUES=1` supplies the same `StableId` to the Markdown section and TSV row.
The spike then terminates with exit `101` at the oracle:

```text
assertion `left != right` failed: stable IDs must differ
left: "synthetic:shared"
right: "synthetic:shared"
```

The committed `red-command.txt` records the same child exit. The normal reproduction command
writes replay output below ignored `target/d1-markdown-tsv-evidence/`, so it cannot alter this
evidence manifest or a frozen candidate.
