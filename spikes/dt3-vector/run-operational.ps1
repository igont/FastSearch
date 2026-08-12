$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$cache = Join-Path $root '.cfknowledge\dt3-b1-pv14'
$model = Join-Path $cache 'models\multilingual-e5-small'
$exe = Join-Path $PSScriptRoot 'target\debug\dt3-vector-spike.exe'
$docs = @(
  (Join-Path $root 'tests\fixtures\dt3\document-root\architecture.md'),
  (Join-Path $root 'tests\fixtures\dt2\guide-current.md'),
  (Join-Path $root 'tests\fixtures\dt2\guide-design.md')
)
if (!(Test-Path -LiteralPath $exe)) { throw 'SPIKE_BINARY_MISSING' }
if (!(Test-Path -LiteralPath $model)) { throw 'E5_CACHE_MISSING' }
if (((Get-Item -LiteralPath $cache -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'CACHE_REPARSE_REJECTED' }
if (([IO.Path]::GetFullPath($cache)).StartsWith(([IO.Path]::GetFullPath((Join-Path $root 'tests\fixtures'))), [StringComparison]::OrdinalIgnoreCase)) { throw 'CACHE_CONTAINMENT_REJECTED' }
$query = 'semantic navigation optional provider fallback'
$runArgs = @('"e5"', ('"' + $model + '"'), ('"' + $query + '"')) + @($docs | ForEach-Object { '"' + $_ + '"' })
$runs = @()
1..10 | ForEach-Object {
  $p = Start-Process -FilePath $exe -ArgumentList $runArgs -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $cache "e5-$_.out") -RedirectStandardError (Join-Path $cache "e5-$_.err")
  $peak = 0L
  while (!$p.HasExited) { $p.Refresh(); $peak = [Math]::Max($peak, $p.PeakWorkingSet64); Start-Sleep -Milliseconds 5 }
  $out = Get-Content -LiteralPath (Join-Path $cache "e5-$_.out") -Raw -Encoding UTF8
  if ($out -notmatch '"dimension":384' -or $out -notmatch '"index":0') { throw "E5_RUN_FAILED_$_ output=$out" }
  $runs += [pscustomobject]@{run=$_; output=$out.Trim(); peak_bytes=$peak}
}
$unique = @($runs.output | ForEach-Object { $_ -replace '"elapsed_ms":\d+', '"elapsed_ms":X' } | Select-Object -Unique)
if ($unique.Count -ne 1) { throw 'E5_ORDER_OR_VECTOR_NONDETERMINISTIC' }
$missingArgs = @('"e5"', ('"' + (Join-Path $cache 'models\missing') + '"'), ('"' + $query + '"')) + @($docs | ForEach-Object { '"' + $_ + '"' })
$missing = Start-Process -FilePath $exe -ArgumentList $missingArgs -NoNewWindow -PassThru -Wait -RedirectStandardOutput (Join-Path $cache 'missing.out') -RedirectStandardError (Join-Path $cache 'missing.err')
if ($missing.ExitCode -eq 0) { throw 'MISSING_CACHE_NOT_REJECTED' }
$latencies = @($runs | ForEach-Object { [int]([regex]::Match($_.output,'"elapsed_ms":(\d+)')).Groups[1].Value } | Sort-Object)
$payload = [ordered]@{schema='dt3-b1-operational-v1'; fixture_root='document-representative-b1-fixture'; cold=$runs[0..4]; warm=$runs[5..9]; p95_ms=$latencies[-1]; peak_working_set_bytes=($runs.peak_bytes | Measure-Object -Maximum).Maximum; missing_cache='rejected'; qwen='unavailable: internal-network loopback route absent'; bge='unavailable: adapter requires three outputs'}
$payload | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $cache 'operational.json') -Encoding UTF8
Write-Output ($payload | ConvertTo-Json -Compress)
