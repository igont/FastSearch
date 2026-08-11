param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('red', 'green')]
    [string]$Mode,
    [string]$EvidenceRoot
)

$ErrorActionPreference = 'Continue'
$PSNativeCommandUseErrorActionPreference = $false
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if ([string]::IsNullOrWhiteSpace($EvidenceRoot)) {
    $EvidenceRoot = Join-Path $repoRoot 'target\d2-tantivy-exact-fts-evidence'
}

New-Item -ItemType Directory -Force -Path $EvidenceRoot | Out-Null
& cargo run --locked --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') -- $Mode 1> (Join-Path $EvidenceRoot "$Mode-run.stdout.txt") 2> (Join-Path $EvidenceRoot "$Mode-run.stderr.txt")
$exitCode = $LASTEXITCODE
Set-Content -LiteralPath (Join-Path $EvidenceRoot "$Mode-command.txt") -Encoding utf8 -Value "powershell -ExecutionPolicy Bypass -File spikes/d2-tantivy-exact-fts/run.ps1 -Mode $Mode`nexit=$exitCode"

if ($Mode -eq 'red' -and $exitCode -eq 0) { exit 1 }
exit $exitCode
