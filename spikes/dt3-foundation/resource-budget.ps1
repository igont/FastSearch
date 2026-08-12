param(
  [Nullable[long]]$NonVectorPeakMiB,
  [Nullable[long]]$VectorChildPeakMiB,
  [Nullable[long]]$StaticModelArtifactBytes,
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

function Assert-Rejected {
  param([scriptblock]$Case, [string]$Expected)
  try {
    & $Case
    throw "RESOURCE_REJECTION_ORACLE_ACCEPTED:$Expected"
  } catch {
    if ($_.Exception.Message -notmatch [regex]::Escape($Expected)) { throw }
  }
}

if ($SelfTest) {
  Assert-ResourceBudget -ObservedNonVectorMiB 1024 -ObservedVectorChildMiB 2048 -ObservedModelArtifactBytes ([long]::MaxValue)
  Assert-Rejected -Expected 'RESOURCE_NON_VECTOR_PEAK_REJECTED' -Case {
    Assert-ResourceBudget -ObservedNonVectorMiB 1025 -ObservedVectorChildMiB 0 -ObservedModelArtifactBytes 0
  }
  Assert-Rejected -Expected 'RESOURCE_VECTOR_CHILD_PEAK_REJECTED' -Case {
    Assert-ResourceBudget -ObservedNonVectorMiB 0 -ObservedVectorChildMiB 2049 -ObservedModelArtifactBytes 0
  }
  Write-Output 'resource split authority, both exact boundaries, both rejection boundaries and uncapped model artifact diagnostic PASS'
  exit 0
}

if ($null -eq $NonVectorPeakMiB -or $null -eq $VectorChildPeakMiB -or $null -eq $StaticModelArtifactBytes) {
  throw 'RESOURCE_MEASUREMENTS_REQUIRED'
}
Assert-ResourceBudget -ObservedNonVectorMiB $NonVectorPeakMiB.Value -ObservedVectorChildMiB $VectorChildPeakMiB.Value -ObservedModelArtifactBytes $StaticModelArtifactBytes.Value
Write-Output 'resource measurements PASS'
