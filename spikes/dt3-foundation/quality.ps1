$ErrorActionPreference='Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$queries = Import-Csv (Join-Path $root 'evidence\dt3\foundation\queries.tsv') -Delimiter "`t"
if ($queries.Count -lt 24) { throw 'QUALITY_ROWS_LT_24' }
foreach($intent in 'exact','lexical','paraphrase','doc-map','symbol','no-hit','vector-unavailable') { if (($queries | Where-Object intent -eq $intent).Count -eq 0) { throw "MISSING_INTENT_$intent" } }
if (($queries.label | Sort-Object -Unique).Count -ne $queries.Count) { throw 'DUPLICATE_LABEL' }
$sameRelative = @('code-fastsearch/src/navigator.rs#symbol=stable_navigation','code-fixture/src/navigator.rs#symbol=stable_navigation')
if (($sameRelative | Sort-Object -Unique).Count -ne 2) { throw 'ROOT_ID_NOT_IN_SOURCE_KEY' }
$fixture = Join-Path $root 'tests\fixtures\dt3'
foreach($item in @('document-root\architecture.md','code-root\src\navigator.rs','code-root\tools\index.py','cfmap\auto.cfmap.md','cfmap\curated.cfmap.md','cfmap\stale.cfmap.md','cfmap\invalid.cfmap.md')) { if (!(Test-Path -LiteralPath (Join-Path $fixture $item))) { throw "MISSING_FIXTURE_$item" } }
Write-Output 'quality labels, intent coverage, deterministic source-key collision and fixture oracle PASS'
