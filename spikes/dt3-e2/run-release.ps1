param(
    [Parameter(Mandatory = $true)][string]$Binary,
    [Parameter(Mandatory = $true)][string]$DocumentRoot,
    [Parameter(Mandatory = $true)][string]$CodeRoot,
    [Parameter(Mandatory = $true)][string]$WorkRoot,
    [Parameter(Mandatory = $true)][string]$OutputJson,
    [string]$E5Root = ''
)

$ErrorActionPreference = 'Stop'
$runId = 'dt3-e2-release-v1'
$resolvedDocument = (Resolve-Path -LiteralPath $DocumentRoot).Path
$resolvedCode = (Resolve-Path -LiteralPath $CodeRoot).Path
$resolvedBinary = (Resolve-Path -LiteralPath $Binary).Path
$resolvedWork = [System.IO.Path]::GetFullPath($WorkRoot)

if ($resolvedWork -eq $resolvedDocument -or $resolvedWork -eq $resolvedCode) {
    throw 'work root must differ from source roots'
}
if ($resolvedDocument.StartsWith($resolvedWork, [System.StringComparison]::OrdinalIgnoreCase) -or
    $resolvedCode.StartsWith($resolvedWork, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw 'work root must not be an ancestor of a source root'
}

New-Item -ItemType Directory -Force -Path $resolvedWork | Out-Null
$marker = Join-Path $resolvedWork 'owner.marker'
[System.IO.File]::WriteAllText($marker, $runId, [System.Text.UTF8Encoding]::new($false))
if ([System.IO.File]::ReadAllText($marker, [System.Text.Encoding]::UTF8) -ne $runId) {
    throw 'run marker readback mismatch'
}

# The approved roots are intentionally tiny functional fixtures. A deterministic
# copied root adds source bytes (not records) so the service-byte ratio measures
# steady-state overhead instead of SQLite's fixed minimum page size.
$scaledDocument = Join-Path $resolvedWork 'scaled-document-root'
Copy-Item -LiteralPath $resolvedDocument -Destination $scaledDocument -Recurse
$scaleText = "# Deterministic release scale`n`n" + ('navigation ' * 131072)
[System.IO.File]::WriteAllText(
    (Join-Path $scaledDocument 'release-scale.md'),
    $scaleText,
    [System.Text.UTF8Encoding]::new($false)
)
$resolvedDocument = (Resolve-Path -LiteralPath $scaledDocument).Path

function Invoke-Measured {
    param([string[]]$Arguments, [string]$Label)
    $stdout = Join-Path $resolvedWork ($Label + '.stdout.txt')
    $stderr = Join-Path $resolvedWork ($Label + '.stderr.txt')
    $watch = [System.Diagnostics.Stopwatch]::StartNew()
    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $resolvedBinary
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    # Windows PowerShell 5.1 has no ProcessStartInfo.ArgumentList. Encode each
    # argv element with the documented CommandLineToArgvW backslash/quote rules.
    $encoded = foreach ($argument in $Arguments) {
        if ($argument -notmatch '[\s"]') { $argument; continue }
        $builder = [System.Text.StringBuilder]::new('"')
        $slashes = 0
        foreach ($character in $argument.ToCharArray()) {
            if ($character -eq '\') { $slashes++; continue }
            if ($character -eq '"') {
                [void]$builder.Append(('\' * ($slashes * 2 + 1)))
                [void]$builder.Append('"')
                $slashes = 0
                continue
            }
            [void]$builder.Append(('\' * $slashes))
            $slashes = 0
            [void]$builder.Append($character)
        }
        [void]$builder.Append(('\' * ($slashes * 2)))
        [void]$builder.Append('"')
        $builder.ToString()
    }
    $start.Arguments = $encoded -join ' '
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $start
    [void]$process.Start()
    $stdoutTask = $process.StandardOutput.ReadToEndAsync()
    $stderrTask = $process.StandardError.ReadToEndAsync()
    $peak = 0L
    while (-not $process.HasExited) {
        $process.Refresh()
        if ($process.PeakWorkingSet64 -gt $peak) { $peak = $process.PeakWorkingSet64 }
        Start-Sleep -Milliseconds 25
    }
    $process.WaitForExit()
    $stdoutText = $stdoutTask.GetAwaiter().GetResult()
    $stderrText = $stderrTask.GetAwaiter().GetResult()
    [System.IO.File]::WriteAllText($stdout, $stdoutText, [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText($stderr, $stderrText, [System.Text.UTF8Encoding]::new($false))
    $process.Refresh()
    if ($process.PeakWorkingSet64 -gt $peak) { $peak = $process.PeakWorkingSet64 }
    $watch.Stop()
    if ($process.ExitCode -ne 0 -or $stderrText.Length -ne 0 -or $stdoutText.Length -eq 0) {
        throw "$Label failed with exit $($process.ExitCode): $stderrText"
    }
    if ($Label -like 'cold-*' -or $Label -like 'vector-cold-*') {
        if ($stdoutText -notmatch 'freshness=Current' -or $stdoutText -notmatch 'CodeMaps=Real' -or $stdoutText -notmatch 'Symbols=Real') {
            throw "$Label semantic status assertion failed"
        }
        if ($Label -like 'vector-cold-*' -and $stdoutText -notmatch 'VectorRetrieval=Real') {
            throw "$Label real vector provider assertion failed"
        }
    } else {
        if ($stdoutText -notmatch 'hits=[1-9]') { throw "$Label semantic search assertion failed" }
    }
    [pscustomobject][ordered]@{
        label = $Label
        milliseconds = [math]::Round($watch.Elapsed.TotalMilliseconds, 3)
        peak_working_set_bytes = $peak
        output_sha256 = (Get-FileHash -LiteralPath $stdout -Algorithm SHA256).Hash.ToLowerInvariant()
        exit_code = $process.ExitCode
    }
}

$cold = @()
for ($index = 1; $index -le 5; $index++) {
    $service = Join-Path $resolvedWork "cold-$index\.cfknowledge"
    $cold += Invoke-Measured -Label "cold-$index" -Arguments @(
        'index', 'rebuild', $resolvedDocument, $resolvedCode, $service
    )
}

$warmService = Join-Path $resolvedWork 'warm\.cfknowledge'
& $resolvedBinary index rebuild $resolvedDocument $resolvedCode $warmService | Out-Null
$warm = @()
for ($index = 1; $index -le 5; $index++) {
    $warm += Invoke-Measured -Label "warm-$index" -Arguments @(
        'search', $resolvedDocument, $resolvedCode, $warmService, 'balanced', 'Navigation'
    )
}

$vector = @()
if ($E5Root -ne '') {
    $resolvedE5 = (Resolve-Path -LiteralPath $E5Root).Path
    for ($index = 1; $index -le 5; $index++) {
        $service = Join-Path $resolvedWork "vector-cold-$index\.cfknowledge"
        $vector += Invoke-Measured -Label "vector-cold-$index" -Arguments @(
            'index', 'rebuild', $resolvedDocument, $resolvedCode, $service, $resolvedE5
        )
    }
    $vectorWarmService = Join-Path $resolvedWork 'vector-warm\.cfknowledge'
    & $resolvedBinary index rebuild $resolvedDocument $resolvedCode $vectorWarmService $resolvedE5 | Out-Null
    for ($index = 1; $index -le 5; $index++) {
        $vector += Invoke-Measured -Label "vector-warm-$index" -Arguments @(
            'search', $resolvedDocument, $resolvedCode, $vectorWarmService, 'balanced', 'Navigation', $resolvedE5
        )
    }
}

$sourceBytes = (Get-ChildItem -LiteralPath $resolvedDocument, $resolvedCode -Recurse -File |
    Measure-Object -Property Length -Sum).Sum
$serviceSizes = Get-ChildItem -LiteralPath $resolvedWork -Directory | ForEach-Object {
    $bytes = (Get-ChildItem -LiteralPath $_.FullName -Recurse -File |
        Where-Object { $_.Name -notlike '*.stdout.txt' -and $_.Name -notlike '*.stderr.txt' } |
        Measure-Object -Property Length -Sum).Sum
    if ($null -eq $bytes) { 0 } else { $bytes }
}
$serviceBytes = ($serviceSizes | Measure-Object -Maximum).Maximum
$nonVectorPeak = (@($cold + $warm) | Measure-Object -Property peak_working_set_bytes -Maximum).Maximum
$vectorPeak = if ($vector.Count -eq 0) { 0 } else {
    ($vector | Measure-Object -Property peak_working_set_bytes -Maximum).Maximum
}
$result = [ordered]@{
    schema = 'dt3-e2-release-v1'
    run_id = $runId
    candidate_revision = (git -C (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) rev-parse HEAD).Trim()
    binary_sha256 = (Get-FileHash -LiteralPath $resolvedBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    rustc = (rustc --version).Trim()
    cargo = (cargo --version).Trim()
    command = 'spikes/dt3-e2/run-release.ps1 release 5 cold + 5 warm + optional E5 5 cold + 5 warm'
    cold = $cold
    warm = $warm
    vector = $vector
    source_bytes = $sourceBytes
    service_bytes = $serviceBytes
    service_ratio = [math]::Round($serviceBytes / [math]::Max(1, $sourceBytes), 6)
    non_vector_peak_bytes = $nonVectorPeak
    vector_peak_bytes = $vectorPeak
    gates = [ordered]@{
        cold_max_ms = (($cold | Measure-Object -Property milliseconds -Maximum).Maximum -le 18000)
        warm_max_ms = (($warm | Measure-Object -Property milliseconds -Maximum).Maximum -le 500)
        non_vector_memory = ($nonVectorPeak -le 1073741824)
        vector_memory = ($vectorPeak -le 2147483648)
        service_ratio = (($serviceBytes / [math]::Max(1, $sourceBytes)) -le 2.0)
        all_samples = ($cold.Count -eq 5 -and $warm.Count -eq 5 -and ($E5Root -eq '' -or $vector.Count -eq 10))
    }
}
$json = $result | ConvertTo-Json -Depth 8
[System.IO.File]::WriteAllText($OutputJson, $json, [System.Text.UTF8Encoding]::new($false))
$readback = Get-Content -LiteralPath $OutputJson -Raw -Encoding UTF8 | ConvertFrom-Json
if ($readback.schema -ne 'dt3-e2-release-v1' -or $readback.run_id -ne $runId) {
    throw 'result readback mismatch'
}
if ($result.gates.Values -contains $false) {
    throw 'one or more release gates failed'
}
$json
