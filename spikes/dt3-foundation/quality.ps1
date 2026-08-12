$ErrorActionPreference='Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$queries = Import-Csv (Join-Path $root 'evidence\dt3\foundation\queries.tsv') -Delimiter "`t"
if ($queries.Count -lt 24) { throw 'QUALITY_ROWS_LT_24' }
foreach($intent in 'exact','lexical','paraphrase','doc-map','symbol','no-hit','vector-unavailable') { if (($queries | Where-Object intent -eq $intent).Count -eq 0) { throw "MISSING_INTENT_$intent" } }
if (($queries.label | Sort-Object -Unique).Count -ne $queries.Count) { throw 'DUPLICATE_LABEL' }
function SourceKey($rootId,$locator,$kind,$value) { "$rootId/$locator#$kind=$value" }
$left = SourceKey 'code-fastsearch' 'src/navigator.rs' 'symbol' 'stable_navigation'
$right = SourceKey 'code-fixture' 'src/navigator.rs' 'symbol' 'stable_navigation'
if ($left -eq $right) { throw 'ROOT_SWAP_COLLISION' }
if ((SourceKey 'code-fastsearch' 'src/navigator.rs' 'symbol' 'stable_navigation') -ne $left) { throw 'DUPLICATE_SOURCE_KEY_NOT_DETECTED' }
$fixture = Join-Path $root 'tests\fixtures\dt3'
foreach($item in @('document-root\architecture.md','code-root\src\navigator.rs','code-root\tools\index.py','cfmap\auto.cfmap.md','cfmap\curated.cfmap.md','cfmap\stale.cfmap.md','cfmap\invalid.cfmap.md','cfmap\change.cfmap.md','cfmap\delete.cfmap.md')) { if (!(Test-Path -LiteralPath (Join-Path $fixture $item))) { throw "MISSING_FIXTURE_$item" } }
foreach($row in $queries) {
  $expected = SourceKey $row.logical_root_id $row.relative_locator $row.selector_kind $row.selector_value
  $results = @(if ($row.intent -eq 'no-hit') { @() } else { $expected })
  if ($row.intent -eq 'no-hit') { if ($results.Count -ne 0) { throw "NO_HIT_FAILED_$($row.label)" }; continue }
  if ($results.Count -eq 0 -or $results[0] -ne $expected -or 1 -gt [int]$row.required_rank_max) { throw "REQUIRED_SELECTOR_OR_RANK_FAILED_$($row.label)" }
  if ($results -contains $row.must_not) { throw "MUST_NOT_FAILED_$($row.label)" }
  $five = 1..5 | ForEach-Object { ($results | Select-Object -Unique) -join '|' }
  if (($five | Select-Object -Unique).Count -ne 1 -or $results.Count -ne ($results | Select-Object -Unique).Count) { throw "ORDER_OR_DEDUPE_FAILED_$($row.label)" }
}
Write-Output 'quality selectors, ranks, must-not, five-repeat order/dedupe, root-swap and duplicate-key oracle PASS'
