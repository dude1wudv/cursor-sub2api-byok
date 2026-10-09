param([string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $projectRoot 'dist\windows-x64' }
if (-not [IO.Path]::IsPathFullyQualified($OutputDirectory)) { throw 'OutputDirectory must be absolute' }
function Invoke-Checked([scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "Build command failed: $Command (exit $LASTEXITCODE)" }
}
Push-Location (Join-Path $projectRoot 'apps\desktop')
try {
    $pnpmVersion = & pnpm --version
    if ($pnpmVersion -notmatch '^9\.') { throw "pnpm 9 required; found $pnpmVersion" }
    Invoke-Checked { pnpm install --frozen-lockfile }
    Invoke-Checked { pnpm run typecheck }
    Invoke-Checked { pnpm run typecheck:node }
    Invoke-Checked { pnpm exec tauri build --no-bundle }
} finally { Pop-Location }
$binary = Join-Path $projectRoot 'target\release\cursor-byok-desktop.exe'
if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw "Missing $binary" }
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
Invoke-Checked { python -X utf8 (Join-Path $PSScriptRoot 'third-party-notices.py') $OutputDirectory }
Copy-Item -LiteralPath $binary -Destination (Join-Path $OutputDirectory 'Cursor-Sub2API-BYOK.exe')
Copy-Item -LiteralPath (Join-Path $projectRoot 'LICENSE') -Destination (Join-Path $OutputDirectory 'LICENSE')
$files = @('Cursor-Sub2API-BYOK.exe','LICENSE','THIRD-PARTY-NOTICES.txt')
$hashes = foreach ($name in $files) {
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $OutputDirectory $name)).Hash.ToLowerInvariant()
    "$hash  $name"
}
[IO.File]::WriteAllLines((Join-Path $OutputDirectory 'SHA256SUMS.txt'), $hashes, [Text.UTF8Encoding]::new($false))
$archive = Join-Path (Split-Path $OutputDirectory -Parent) 'Cursor-Sub2API-BYOK-windows-x64.zip'
$packageFiles = @($files + 'SHA256SUMS.txt' | ForEach-Object { Join-Path $OutputDirectory $_ })
Compress-Archive -LiteralPath $packageFiles -DestinationPath $archive -Force
Write-Output "Portable package: $archive"
