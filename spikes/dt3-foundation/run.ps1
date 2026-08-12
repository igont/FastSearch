[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$DocumentRoot,
  [Parameter(Mandatory)][string]$CodeRoot,
  [Parameter(Mandatory)][string]$ServiceRoot,
  [Parameter(Mandatory)][ValidatePattern('^[a-z0-9][a-z0-9-]{2,63}$')][string]$RunId,
  [switch]$NegativeOnly
)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
function Canonical([string]$Path) { [IO.Path]::GetFullPath($Path).TrimEnd([char]92,[char]47) }
function IsDescendant([string]$Child,[string]$Parent) { $Child.StartsWith($Parent + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase) }
function HasReparse([string]$Path) {
  $p = Canonical $Path
  while ($true) { if (Test-Path -LiteralPath $p) { if (((Get-Item -LiteralPath $p -Force).Attributes -band [IO.FileAttributes]::ReparsePoint)) { return $true } }; $next=[IO.Directory]::GetParent($p); if ($null -eq $next) { return $false }; $p=$next.FullName }
}
function AssertSafeService([string]$Doc,[string]$Code,[string]$Service,[string]$Id) {
  $d=Canonical $Doc; $c=Canonical $Code; $s=Canonical $Service
  if (!(Test-Path -LiteralPath $d) -or !(Test-Path -LiteralPath $c)) { throw 'INPUT_ROOT_MISSING' }
  if ($s -eq $d -or $s -eq $c -or (IsDescendant $d $s) -or (IsDescendant $c $s)) { throw 'SERVICE_OVERLAP_REJECTED' }
  foreach($root in @($d,$c)) { if ((IsDescendant $s $root)) { $allowed = Join-Path $root ('.cfknowledge\dt3-a1-' + $Id); if ($s -ne (Canonical $allowed)) { throw 'SERVICE_DESCENDANT_NOT_EXACT_RUN_ZONE' } } }
  if ((HasReparse $s)) { throw 'SERVICE_REPARSE_ESCAPE_REJECTED' }
  return $s
}
function AssertRedacted([object]$Payload) {
  $json=$Payload | ConvertTo-Json -Depth 8 -Compress
  if ($json -match '(?i)([A-Z]:\\\\|/Users/|token=|password=|secret=|content|snippet)') { throw 'EVIDENCE_REDACTION_REJECTED' }
}
if ($NegativeOnly) { exit 0 }
$service=AssertSafeService $DocumentRoot $CodeRoot $ServiceRoot $RunId
New-Item -ItemType Directory -Force -Path $service | Out-Null
$marker=Join-Path $service 'dt3-a1-run.marker'; Set-Content -LiteralPath $marker -Value $RunId -Encoding utf8NoBOM
$payload=[ordered]@{ schema='dt3-a1-redacted-v1'; run_id=$RunId; roots=@('document-representative','code-fastsearch'); marker_sha256=(Get-FileHash -Path $marker -Algorithm SHA256).Hash.ToLowerInvariant(); error_classes=@(); timings_ms=@{} }
AssertRedacted $payload
$out=Join-Path $service 'run.json'; $payload | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath $out -Encoding utf8NoBOM
if (!(Test-Path -LiteralPath $out) -or (Get-Content -LiteralPath $marker -Raw -Encoding utf8).Trim() -ne $RunId) { throw 'READBACK_FAILED' }
Write-Output ($payload | ConvertTo-Json -Compress)
