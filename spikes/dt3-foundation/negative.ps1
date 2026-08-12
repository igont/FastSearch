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
Write-Output 'negative containment cases rejected before write'
