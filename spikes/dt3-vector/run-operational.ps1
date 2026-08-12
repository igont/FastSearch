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
$queries = Import-Csv (Join-Path $root 'evidence\dt3\foundation\queries.tsv') -Delimiter "`t"
$applicable = @($queries | Where-Object { $_.section -eq 'A1/B1' -and $_.intent -eq 'paraphrase' })
if ($applicable.Count -lt 1 -or @($applicable | Where-Object { $_.relative_locator -eq 'architecture.md' -and $_.must_not -eq 'secret sentinel' }).Count -lt 1) { throw 'A1_QUERY_MAPPING_MISSING' }
$manifest = @()
foreach($spec in @(
  @{name='e5'; revision='614241f622f53c4eeff9890bdc4f31cfecc418b3'; file='models\multilingual-e5-small\onnx\model.onnx'; hash='CA456C06B3A9505D'},
  @{name='bge'; revision='5617a9f61b028005a4858fdac845db406aefb181'; file='models\bge-m3\onnx\model.onnx_data'; hash='1EEBFB28493F67BB'},
  @{name='qwen'; revision='97b0c614be4d77ee51c0cef4e5f07c00f9eb65b3'; file='models\qwen3-embedding-0.6b\model.safetensors'; hash='0437E45C94563B09'}
)) {
  $path = Join-Path $cache $spec.file; $item=Get-Item -LiteralPath $path; $hash=(Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash
  if (!$hash.StartsWith($spec.hash)) { throw "MODEL_HASH_MISMATCH_$($spec.name)" }
  $manifest += [ordered]@{name=$spec.name; revision=$spec.revision; sha256=$hash; bytes=$item.Length}
}
$runArgs = @('"e5"', ('"' + $model + '"'), ('"' + $query + '"')) + @($docs | ForEach-Object { '"' + $_ + '"' })
$runs = @()
1..10 | ForEach-Object {
  $p = Start-Process -FilePath $exe -ArgumentList $runArgs -NoNewWindow -PassThru -RedirectStandardOutput (Join-Path $cache "e5-$_.out") -RedirectStandardError (Join-Path $cache "e5-$_.err")
  $peak = 0L
  while (!$p.HasExited) { $p.Refresh(); $peak = [Math]::Max($peak, $p.PeakWorkingSet64); Start-Sleep -Milliseconds 5 }
  $out = Get-Content -LiteralPath (Join-Path $cache "e5-$_.out") -Raw -Encoding UTF8
  if ($out -notmatch '"dimension":384' -or $out -notmatch '"norm":' -or $out -notmatch '"batch_size":1' -or $out -notmatch '"index":0') { throw "E5_RUN_FAILED_$_ output=$out" }
  $runs += [pscustomobject]@{run=$_; output=$out.Trim(); peak_bytes=$peak}
}
$unique = @($runs.output | ForEach-Object { $_ -replace '"elapsed_ms":\d+', '"elapsed_ms":X' } | Select-Object -Unique)
if ($unique.Count -ne 1) { throw 'E5_ORDER_OR_VECTOR_NONDETERMINISTIC' }
if ($runs[0].output -notmatch '"selectors":\["architecture.md#Navigation contract"' -or $runs[0].output -match 'secret sentinel') { throw 'E5_MUST_NOT_SELECTOR_FAILED' }
$missingArgs = @('"e5"', ('"' + (Join-Path $cache 'models\missing') + '"'), ('"' + $query + '"')) + @($docs | ForEach-Object { '"' + $_ + '"' })
$missing = Start-Process -FilePath $exe -ArgumentList $missingArgs -NoNewWindow -PassThru -Wait -RedirectStandardOutput (Join-Path $cache 'missing.out') -RedirectStandardError (Join-Path $cache 'missing.err')
if ($missing.ExitCode -eq 0 -or (Get-Content -LiteralPath (Join-Path $cache 'missing.err') -Raw -Encoding UTF8) -notmatch 'B1_NO_PROVIDER_CACHE_MISSING') { throw 'MISSING_CACHE_NOT_TYPED' }
$recovery = & $exe @runArgs 2>&1
if ($LASTEXITCODE -ne 0 -or $recovery -notmatch '"index":0') { throw 'E5_RECOVERY_FAILED' }
$junction = Join-Path $cache 'junction-probe'
cmd.exe /c "mklink /J `"$junction`" `"$model`"" | Out-Null
if (!((Get-Item -LiteralPath $junction -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'JUNCTION_NOT_CREATED' }
if ((Get-Item -LiteralPath $junction -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { $reparse='rejected' } else { throw 'REPARSE_NOT_REJECTED' }
cmd.exe /c "rmdir `"$junction`"" | Out-Null
if (Test-Path -LiteralPath $junction) { throw 'JUNCTION_CLEANUP_FAILED' }
$latencies = @($runs | ForEach-Object { [int]([regex]::Match($_.output,'"elapsed_ms":(\d+)')).Groups[1].Value } | Sort-Object)
$peak = ($runs.peak_bytes | Measure-Object -Maximum).Maximum
if ($peak -gt 2147483648) { throw 'E5_VECTOR_CHILD_MEMORY_BUDGET_EXCEEDED' }
$fullManifest = foreach($directory in @('models\multilingual-e5-small','models\bge-m3','models\qwen3-embedding-0.6b')) { Get-ChildItem -LiteralPath (Join-Path $cache $directory) -Recurse -File | Where-Object { $_.FullName -notmatch '\\.git\\' } | ForEach-Object { [ordered]@{profile=$directory; locator=$_.FullName.Substring((Join-Path $cache $directory).Length).TrimStart('\\'); bytes=$_.Length; sha256=(Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash} } }
$oldErrorActionPreference = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
$bge = & $exe bge (Join-Path $cache 'models\bge-m3') $query $docs 2>&1
$bgeExit = $LASTEXITCODE
$ErrorActionPreference = $oldErrorActionPreference
if ($bgeExit -eq 0 -or ($bge | Out-String) -notmatch 'expects the model to return 3 outputs') { throw 'BGE_CAUSAL_FAILURE_NOT_REPRODUCED' }
$qwen = docker inspect fastsearch-dt3-b1-qwen --format '{{json .NetworkSettings.Ports}} {{.HostConfig.ReadonlyRootfs}}' 2>&1
$payload = [ordered]@{schema='dt3-b1-operational-v3'; fixture_root='document-representative-b1-fixture'; query_contract='A1/B1 paraphrase expected architecture.md, must-not secret sentinel'; models=$manifest; full_cache_manifest=$fullManifest; cold=$runs[0..4]; warm=$runs[5..9]; p95_ms=$latencies[-1]; peak_working_set_bytes=$peak; vector_child_budget_bytes=2147483648; missing_cache='typed-rejected'; recovery='same-model-hash accepted'; junction_reparse=$reparse; bge=($bge | Out-String).Trim(); qwen_inspect=($qwen | Out-String).Trim(); qwen='unavailable: internal-network loopback route absent'}
$payload | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $cache 'operational.json') -Encoding UTF8
Write-Output ($payload | ConvertTo-Json -Compress)
