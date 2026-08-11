param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('red', 'green')]
    [string]$Mode,
    [string]$EvidenceRoot
)

$ErrorActionPreference = 'Continue'
$PSNativeCommandUseErrorActionPreference = $false
$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$fixtureRoot = Join-Path $PSScriptRoot 'fixtures'
if ([string]::IsNullOrWhiteSpace($EvidenceRoot)) {
    $EvidenceRoot = Join-Path $repoRoot 'target\d1-markdown-tsv-evidence'
}
$artifact = Join-Path $repoRoot 'target\debug\libfastsearch.rlib'
$binary = Join-Path $repoRoot 'target\debug\d1-spike.exe'

New-Item -ItemType Directory -Force -Path $EvidenceRoot | Out-Null
& cargo build --locked --lib --manifest-path (Join-Path $repoRoot 'Cargo.toml') 1> (Join-Path $EvidenceRoot "$Mode-build.stdout.txt") 2> (Join-Path $EvidenceRoot "$Mode-build.stderr.txt")
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
& rustc --edition=2024 (Join-Path $PSScriptRoot 'main.rs') --extern "fastsearch=$artifact" -L (Join-Path $repoRoot 'target\debug') -o $binary 1> (Join-Path $EvidenceRoot "$Mode-compile.stdout.txt") 2> (Join-Path $EvidenceRoot "$Mode-compile.stderr.txt")
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

if ($Mode -eq 'red') { $env:D1_FORCE_SHARED_VALUES = '1' }
& $binary $fixtureRoot 1> (Join-Path $EvidenceRoot "$Mode-run.stdout.txt") 2> (Join-Path $EvidenceRoot "$Mode-run.stderr.txt")
$exitCode = $LASTEXITCODE
Remove-Item Env:D1_FORCE_SHARED_VALUES -ErrorAction SilentlyContinue
Set-Content -LiteralPath (Join-Path $EvidenceRoot "$Mode-command.txt") -Encoding utf8 -Value "powershell -ExecutionPolicy Bypass -File spikes/d1-markdown-tsv/run.ps1 -Mode $Mode`nexit=$exitCode"

if ($Mode -eq 'red' -and $exitCode -eq 0) { exit 1 }
exit $exitCode
