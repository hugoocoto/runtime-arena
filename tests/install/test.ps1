# Runs install.ps1 against a fake release, with no network: a stub
# Invoke-WebRequest serves the files. $env:ARENA_EXE is a real arena.exe to package.
$ErrorActionPreference = 'Stop'

$root = Resolve-Path (Join-Path $PSScriptRoot '../..')
$work = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
$release = Join-Path $work 'release'
$name = 'arena-v9.9.9-x86_64-pc-windows-msvc'
$asset = "$name.zip"
New-Item -ItemType Directory -Path (Join-Path $work $name), $release | Out-Null

# A function takes precedence over the cmdlet of the same name in install.ps1.
function Invoke-WebRequest {
    param([switch] $UseBasicParsing, [string] $Uri, [string] $OutFile)
    Copy-Item -LiteralPath (Join-Path $release ($Uri -split '/')[-1]) -Destination $OutFile
}

function Install-To($directory) {
    $env:VERSION = '9.9.9'
    $env:PREFIX = Join-Path $work $directory
    & (Join-Path $root 'install.ps1') | Out-Null
    $env:PREFIX
}

try {
    Copy-Item -LiteralPath $env:ARENA_EXE -Destination (Join-Path (Join-Path $work $name) 'arena.exe')
    $zip = Join-Path $release $asset
    Compress-Archive -Path (Join-Path $work $name) -DestinationPath $zip
    $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash.ToLower()
    $sums = Join-Path $release 'SHA256SUMS'

    Set-Content -LiteralPath $sums -Value "$hash  $asset"
    $prefix = Install-To 'good'
    if (-not (Test-Path -LiteralPath (Join-Path $prefix 'arena.exe'))) { throw 'arena.exe was not installed' }
    Write-Host 'ok    clean install'

    Set-Content -LiteralPath $sums -Value "$('0' * 64)  $asset"
    $refused = $false
    try { Install-To 'tampered' | Out-Null } catch { $refused = $_.Exception.Message -like 'checksum mismatch*' }
    if (-not $refused -or (Test-Path -LiteralPath (Join-Path $work 'tampered/arena.exe'))) {
        throw 'installed despite a checksum mismatch'
    }
    Write-Host 'ok    checksum mismatch refused'

    Set-Content -LiteralPath $sums -Value "$hash  another-file.zip"
    $refused = $false
    try { Install-To 'unlisted' | Out-Null } catch { $refused = $_.Exception.Message -like 'SHA256SUMS has no entry*' }
    if (-not $refused -or (Test-Path -LiteralPath (Join-Path $work 'unlisted/arena.exe'))) {
        throw 'installed an asset missing from SHA256SUMS'
    }
    Write-Host 'ok    unlisted asset refused'
} finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}
