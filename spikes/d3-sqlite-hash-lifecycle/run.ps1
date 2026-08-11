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
    $EvidenceRoot = Join-Path $repoRoot 'target\d3-sqlite-hash-lifecycle-evidence'
}
$database = Join-Path $EvidenceRoot "$Mode-lifecycle.sqlite"

New-Item -ItemType Directory -Force -Path $EvidenceRoot | Out-Null
Remove-Item -LiteralPath $database -Force -ErrorAction SilentlyContinue
& cargo run --locked --manifest-path (Join-Path $PSScriptRoot 'Cargo.toml') -- $Mode $database 1> (Join-Path $EvidenceRoot "$Mode-run.stdout.txt") 2> (Join-Path $EvidenceRoot "$Mode-run.stderr.txt")
$exitCode = $LASTEXITCODE
$databaseExisted = Test-Path -LiteralPath $database
Remove-Item -LiteralPath $database -Force -ErrorAction SilentlyContinue
$databaseRemoved = -not (Test-Path -LiteralPath $database)
Set-Content -LiteralPath (Join-Path $EvidenceRoot "$Mode-command.txt") -Encoding utf8 -Value "powershell -ExecutionPolicy Bypass -File spikes/d3-sqlite-hash-lifecycle/run.ps1 -Mode $Mode`nexit=$exitCode`ndatabase_existed_before_cleanup=$databaseExisted`ndatabase_removed=$databaseRemoved"

if (-not $databaseRemoved) { exit 1 }
if ($Mode -eq 'red' -and $exitCode -eq 0) { exit 1 }
exit $exitCode
