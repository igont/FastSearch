param(
  [long]$NonVectorPeakMiB,
  [long]$VectorChildPeakMiB,
  [long]$StaticModelArtifactBytes,
  [switch]$SelfTest
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$manifestPath = Join-Path $root 'evidence\dt3\foundation\manifest.json'
$manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
$nonVectorLimit = [long]$manifest.performance.working_set_mib.non_vector_max
$vectorChildLimit = [long]$manifest.performance.working_set_mib.local_vector_child_max

if ($nonVectorLimit -ne 1024) { throw 'RESOURCE_MANIFEST_NON_VECTOR_AUTHORITY_MISMATCH' }
if ($vectorChildLimit -ne 2048) { throw 'RESOURCE_MANIFEST_VECTOR_CHILD_AUTHORITY_MISMATCH' }
if ($manifest.performance.static_model_artifact_bytes.classification -ne 'diagnostic' -or
    $manifest.performance.static_model_artifact_bytes.counts_toward_working_set -ne $false -or
    $null -ne $manifest.performance.static_model_artifact_bytes.max) {
  throw 'RESOURCE_MANIFEST_MODEL_ARTIFACT_AUTHORITY_MISMATCH'
}

function Assert-ResourceBudget {
  param(
    [long]$ObservedNonVectorMiB,
    [long]$ObservedVectorChildMiB,
    [long]$ObservedModelArtifactBytes
  )
  if ($ObservedNonVectorMiB -lt 0 -or $ObservedVectorChildMiB -lt 0 -or $ObservedModelArtifactBytes -lt 0) {
    throw 'RESOURCE_NEGATIVE_MEASUREMENT_REJECTED'
  }
  if ($ObservedNonVectorMiB -gt $nonVectorLimit) {
    throw "RESOURCE_NON_VECTOR_PEAK_REJECTED:$ObservedNonVectorMiB>$nonVectorLimit"
  }
  if ($ObservedVectorChildMiB -gt $vectorChildLimit) {
    throw "RESOURCE_VECTOR_CHILD_PEAK_REJECTED:$ObservedVectorChildMiB>$vectorChildLimit"
  }
}

function Invoke-PublicCase {
  param(
    [long]$ObservedNonVectorMiB,
    [long]$ObservedVectorChildMiB,
    [long]$ObservedModelArtifactBytes,
    [int]$ExpectedExit,
    [string]$ExpectedOutput
  )
  $stdoutPath = [System.IO.Path]::GetTempFileName()
  $stderrPath = [System.IO.Path]::GetTempFileName()
  try {
    $arguments = @(
      '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', ('"' + $PSCommandPath + '"'),
      '-NonVectorPeakMiB', $ObservedNonVectorMiB,
      '-VectorChildPeakMiB', $ObservedVectorChildMiB,
      '-StaticModelArtifactBytes', $ObservedModelArtifactBytes
    )
    $process = Start-Process -FilePath 'powershell' -ArgumentList $arguments -NoNewWindow -Wait -PassThru `
      -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
    $output = ((Get-Content -LiteralPath $stdoutPath -Raw -ErrorAction SilentlyContinue) +
      (Get-Content -LiteralPath $stderrPath -Raw -ErrorAction SilentlyContinue))
    if (($ExpectedExit -eq 0 -and $process.ExitCode -ne 0) -or ($ExpectedExit -ne 0 -and $process.ExitCode -eq 0)) {
      throw "RESOURCE_PUBLIC_EXIT_MISMATCH:expected=$ExpectedExit actual=$($process.ExitCode) output=$output"
    }
    if ($output -notmatch [regex]::Escape($ExpectedOutput)) {
      throw "RESOURCE_PUBLIC_OUTPUT_MISMATCH:expected=$ExpectedOutput output=$output"
    }
  } finally {
    Remove-Item -LiteralPath $stdoutPath, $stderrPath -Force -ErrorAction SilentlyContinue
  }
}

if ($SelfTest) {
  Invoke-PublicCase -ObservedNonVectorMiB 1024 -ObservedVectorChildMiB 2048 -ObservedModelArtifactBytes ([long]::MaxValue) -ExpectedExit 0 -ExpectedOutput 'resource measurements PASS'
  Invoke-PublicCase -ObservedNonVectorMiB 1025 -ObservedVectorChildMiB 0 -ObservedModelArtifactBytes 0 -ExpectedExit 1 -ExpectedOutput 'RESOURCE_NON_VECTOR_PEAK_REJECTED'
  Invoke-PublicCase -ObservedNonVectorMiB 0 -ObservedVectorChildMiB 2049 -ObservedModelArtifactBytes 0 -ExpectedExit 1 -ExpectedOutput 'RESOURCE_VECTOR_CHILD_PEAK_REJECTED'
  Write-Output 'resource split authority, both exact boundaries, both rejection boundaries and uncapped model artifact diagnostic PASS'
  exit 0
}

if (-not $PSBoundParameters.ContainsKey('NonVectorPeakMiB') -or
    -not $PSBoundParameters.ContainsKey('VectorChildPeakMiB') -or
    -not $PSBoundParameters.ContainsKey('StaticModelArtifactBytes')) {
  throw 'RESOURCE_MEASUREMENTS_REQUIRED'
}
Assert-ResourceBudget -ObservedNonVectorMiB $NonVectorPeakMiB -ObservedVectorChildMiB $VectorChildPeakMiB -ObservedModelArtifactBytes $StaticModelArtifactBytes
Write-Output 'resource measurements PASS'
