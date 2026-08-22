param(
    [string]$EvidenceRoot = "evidence/dt4"
)

$ErrorActionPreference = "Stop"
$roles = @(
    Get-Content -Raw -LiteralPath (Join-Path $EvidenceRoot "resource-arctic.json") | ConvertFrom-Json
    Get-Content -Raw -LiteralPath (Join-Path $EvidenceRoot "resource-e5-large.json") | ConvertFrom-Json
    Get-Content -Raw -LiteralPath (Join-Path $EvidenceRoot "resource-nomic.json") | ConvertFrom-Json
)
$qwen = Get-Content -Raw -LiteralPath (Join-Path $EvidenceRoot "qwen-preflight.json") | ConvertFrom-Json
$queue = Get-Content -Raw -LiteralPath (Join-Path $EvidenceRoot "resource-queue.json") | ConvertFrom-Json

$result = [ordered]@{
    schema = 1
    gate = "G-RESOURCE-PREFLIGHT@A2"
    status = "PASS"
    thresholds_assigned = $false
    environment = [ordered]@{
        os = "Windows x86_64"
        rustc = "1.95.0"
        cargo = "1.95.0"
    }
    embedding_roles = $roles
    qwen_role = [ordered]@{
        model_revision = $qwen.model_revision
        cold_open_ms = $qwen.cold_open_ms
        inference_ms = $qwen.inference_ms
        working_set_after_open_bytes = $qwen.working_set_after_open_bytes
        working_set_after_inference_bytes = $qwen.working_set_after_inference_bytes
        free_physical_memory_before_bytes = $qwen.free_physical_memory_before_bytes
        free_physical_memory_after_bytes = $qwen.free_physical_memory_after_bytes
        pair_count = $qwen.rust_scores.Count
        maximum_absolute_delta = $qwen.maximum_absolute_delta
    }
    queue_and_response = $queue
    note = "Values are measurements only. A2 does not establish acceptable limits."
}

$json = $result | ConvertTo-Json -Depth 20
[System.IO.File]::WriteAllText(
    (Join-Path (Resolve-Path $EvidenceRoot) "resource-preflight.json"),
    $json.Replace("`r`n", "`n") + "`n",
    [System.Text.UTF8Encoding]::new($false)
)
