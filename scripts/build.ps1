param([string]$Domain = "api.atlassian.com")
$ErrorActionPreference = 'Stop'
if ($Domain -notmatch '^(?=.{1,253}$)[a-z0-9]+(?:[.-][a-z0-9]+)*\.[a-z0-9]+$') { throw 'Invalid API domain' }
$root = Split-Path $PSScriptRoot -Parent

function Assert-Build($condition, $message) {
    if (-not $condition) { throw $message }
}

$manifest = Get-Content -LiteralPath "$root/manifest.json" -Raw -Encoding UTF8 | ConvertFrom-Json
Assert-Build ($manifest.id -ceq 'jira-tickets') 'Unexpected plugin ID'
Assert-Build ($manifest.api_version -ceq '1.3.0') 'Unexpected API version'
Assert-Build ($manifest.storage_schema_version -eq 1) 'Unexpected storage schema version'
Assert-Build ($manifest.allow_custom_domains -eq $true) 'Custom API domains must be enabled'
Assert-Build ($manifest.http_credentials.Count -eq 2) 'Expected two credential rules'
foreach ($prefix in @('/rest/api/2', '/rest/api/3')) {
    $credential = @($manifest.http_credentials | Where-Object { $_.path_prefix -ceq $prefix })
    Assert-Build ($credential.Count -eq 1 -and $credential[0].destination.type -ceq 'base_url' -and $credential[0].destination.setting -ceq 'base_url' -and $credential[0].bearer_secret -ceq 'token') 'Credentials must be scoped to v2 and v3 Base URL paths'
}
$api = $manifest.settings | Where-Object { $_.key -ceq 'api' }
Assert-Build ($api.default -ceq 'data-center' -and $api.options.Count -eq 2 -and $api.options[0].value -ceq 'data-center' -and $api.options[1].value -ceq 'cloud') 'Self-hosted Jira must be the default, with Cloud also supported'
$wit = Get-Content -LiteralPath "$root/cardbe-plugin-api/wit/plugin.wit" -Raw -Encoding UTF8
$witVersion = [regex]::Match($wit, '(?m)^package cardbe:plugin@([^;]+);').Groups[1].Value
Assert-Build ($witVersion -ceq $manifest.api_version) 'WIT package version must match manifest.api_version'
$cargo = Get-Content -LiteralPath "$root/Cargo.toml" -Raw -Encoding UTF8
$cargoVersion = [regex]::Match($cargo, '(?m)^version = "([^"]+)"').Groups[1].Value
Assert-Build ($cargoVersion -ceq $manifest.version) 'Cargo version must match manifest.version'
if ($env:GITHUB_REF_TYPE -ceq 'tag') {
    Assert-Build ($env:GITHUB_REF_NAME -ceq "v$($manifest.version)") 'Release tag must match manifest.version'
}

Push-Location $root
try {
    New-Item -ItemType Directory -Path dist -Force | Out-Null
    cargo build --locked --release --target wasm32-unknown-unknown --target-dir "$root/target"
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed with exit code $LASTEXITCODE" }
    wasm-tools component new target/wasm32-unknown-unknown/release/cardbe_jira_tickets.wasm --output dist/component.wasm --validate
    if ($LASTEXITCODE -ne 0) { throw "wasm-tools failed with exit code $LASTEXITCODE" }
    $component = [System.IO.File]::ReadAllBytes("$root/dist/component.wasm")
    Assert-Build ($component.Length -ge 8) 'Component header is truncated'
    Assert-Build ([BitConverter]::ToString($component, 0, 8) -ceq '00-61-73-6D-0D-00-01-00') 'Invalid Component header'
    Assert-Build ($component.Length -le 16MB) 'Component exceeds 16 MiB'
    $manifest.domains = @($Domain)
    [IO.File]::WriteAllText("$root/dist/manifest.json", ($manifest | ConvertTo-Json -Depth 20), (New-Object System.Text.UTF8Encoding($false)))
    $archive = "cardbe-plugin-jira-tickets-$($manifest.version).zip"
    Compress-Archive -LiteralPath dist/manifest.json, dist/component.wasm -DestinationPath $archive -Force
    $hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    [System.IO.File]::WriteAllText("$root/$archive.sha256", "$hash  $archive`n", (New-Object System.Text.UTF8Encoding($false)))
    Write-Host "Built installable package: $root/dist"
    Write-Host "Built release archive: $root/$archive"
} finally {
    Pop-Location
}

