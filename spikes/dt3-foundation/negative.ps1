$doc = Join-Path $PSScriptRoot '..\\..\\tests\\fixtures\\dt3\\document-root'
$code = Join-Path $PSScriptRoot '..\\..\\tests\\fixtures\\dt3\\code-root'
$run = Join-Path $PSScriptRoot 'run.ps1'
$cases = @(
  @{ name='equal'; service=$doc },
  @{ name='ancestor'; service=(Split-Path -Parent $doc) },
  @{ name='wrong-descendant'; service=(Join-Path $doc '.cfknowledge\\not-run') }
)
foreach($case in $cases) { try { & $run -DocumentRoot $doc -CodeRoot $code -ServiceRoot $case.service -RunId 'safe-run-01' | Out-Null; throw "negative case accepted: $($case.name)" } catch { if ($_.Exception.Message -notmatch 'SERVICE_') { throw } } }
foreach($payload in @('C:\forbidden\raw.md','source snippet content','token=sentinel')) { try { & $run -DocumentRoot $doc -CodeRoot $code -ServiceRoot $doc -RunId 'safe-run-01' -EvidencePayload $payload | Out-Null; throw 'redaction case accepted' } catch { if ($_.Exception.Message -notmatch 'EVIDENCE_REDACTION_REJECTED') { throw } } }
$reparseRunId = 'reparse-run-02'
$junction = Join-Path $doc ('.cfknowledge\dt3-a1-' + $reparseRunId)
$junctionParent = Split-Path -Parent $junction
$createdParent = !(Test-Path -LiteralPath $junctionParent)
if (Test-Path -LiteralPath $junction) { throw 'REPARSE_TEST_TARGET_ALREADY_EXISTS' }
try {
  New-Item -ItemType Junction -Path $junction -Target $PSScriptRoot | Out-Null
  try { & $run -DocumentRoot $doc -CodeRoot $code -ServiceRoot $junction -RunId $reparseRunId | Out-Null; throw 'reparse case accepted' } catch { if ($_.Exception.Message -notmatch 'SERVICE_REPARSE_ESCAPE_REJECTED') { throw } }
} finally { if (Test-Path -LiteralPath $junction) { [System.IO.Directory]::Delete($junction, $false) }; if ($createdParent -and (Test-Path -LiteralPath $junctionParent) -and (Get-ChildItem -LiteralPath $junctionParent -Force | Measure-Object).Count -eq 0) { [System.IO.Directory]::Delete($junctionParent, $false) } }
Write-Output 'negative containment cases rejected before write'
