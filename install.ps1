# Downloads the arena CLI for Windows, verifies its checksum and installs it.
#
#   irm https://raw.githubusercontent.com/hugoocoto/runtime-arena/main/install.ps1 | iex
#
# $env:PREFIX   install directory (default %LOCALAPPDATA%\Programs\arena)
# $env:VERSION  release to install, e.g. 0.6.0 (default: the latest)

# A script block, and `throw` rather than `exit`: under `iex` this runs in the
# user's session, which must neither keep these variables nor be closed.
& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'
    # Windows PowerShell 5.1 may not enable TLS 1.2 by default.
    [Net.ServicePointManager]::SecurityProtocol =
        [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $repo = 'hugoocoto/runtime-arena'
    # Windows on Arm runs the x86-64 build under emulation.
    if ($env:PROCESSOR_ARCHITECTURE -notin 'AMD64', 'ARM64') {
        throw "no prebuilt CLI for $env:PROCESSOR_ARCHITECTURE"
    }
    $target = 'x86_64-pc-windows-msvc'

    $prefix = $env:PREFIX
    if (-not $prefix) {
        if (-not $env:LOCALAPPDATA) { throw 'LOCALAPPDATA is not set; set $env:PREFIX' }
        $prefix = Join-Path $env:LOCALAPPDATA 'Programs\arena'
    }

    if ($env:VERSION) {
        $tag = 'v' + $env:VERSION.TrimStart('v')
    } else {
        # From the "latest" redirect: the GitHub API would rate-limit a room of
        # participants behind one address.
        $response = [Net.WebRequest]::Create("https://github.com/$repo/releases/latest").GetResponse()
        try { $tag = $response.ResponseUri.AbsolutePath.Split('/')[-1] } finally { $response.Close() }
    }
    if ($tag -notmatch '^v[0-9]') { throw 'cannot find the latest release' }

    $name = "arena-$tag-$target"
    $asset = "$name.zip"
    $base = "https://github.com/$repo/releases/download/$tag"
    $work = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
    New-Item -ItemType Directory -Path $work | Out-Null

    try {
        Write-Host "Downloading $name"
        $sums = Join-Path $work 'SHA256SUMS'
        try {
            Invoke-WebRequest -UseBasicParsing -Uri "$base/SHA256SUMS" -OutFile $sums
        } catch {
            throw "release $tag has no SHA256SUMS, so it can't be verified; this installer needs a newer release"
        }
        $zip = Join-Path $work $asset
        Invoke-WebRequest -UseBasicParsing -Uri "$base/$asset" -OutFile $zip

        $expected = $null
        foreach ($line in Get-Content -LiteralPath $sums) {
            $fields = -split $line
            if ($fields.Count -eq 2 -and $fields[1] -ceq $asset) { $expected = $fields[0] }
        }
        if (-not $expected) { throw "SHA256SUMS has no entry for $asset" }
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $zip).Hash
        if ($actual -ine $expected) { throw "checksum mismatch for $asset" }

        Expand-Archive -LiteralPath $zip -DestinationPath $work
        New-Item -ItemType Directory -Force -Path $prefix | Out-Null
        Copy-Item -LiteralPath (Join-Path (Join-Path $work $name) 'arena.exe') -Destination $prefix -Force
    } finally {
        Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
    }

    Write-Host "Installed arena $tag to $(Join-Path $prefix 'arena.exe')"
    $onPath = ($env:PATH -split [IO.Path]::PathSeparator) -contains $prefix
    if (-not $onPath) {
        Write-Host 'Add it to your PATH: Settings > Environment Variables > Path, then add:'
        Write-Host "  $prefix"
        $env:Path = "$prefix$([IO.Path]::PathSeparator)$env:Path"
    }
    & (Join-Path $prefix 'arena.exe') --version
}
