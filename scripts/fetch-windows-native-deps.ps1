<#
.SYNOPSIS
Fetches GTK 4 and its full runtime dependency closure for Windows x64 from MSYS2's UCRT64
repository, bundles it into src/CSGtk.Gtk.Native.Windows/runtimes/win-x64/native/, and packs
the NuGet package into src/local-packages/.

.DESCRIPTION
Rerun this whenever you want to refresh to a newer GTK4/MSYS2 release - it always re-resolves
the dependency graph from the live repository rather than a pinned list.

Unlike fetch-windows-native-deps.sh (which additionally verifies the bundle via a real PE
import-table closure using objdump - only available in an MSYS2/binutils shell), this script
bundles every DLL from every package MSYS2's own dependency metadata resolves, without further
trimming. That's a larger bundle than strictly necessary (a handful of unrelated packages, e.g.
Tcl/Tk/Python pulled in as optional GTK doc-tooling dependencies, ship DLLs that GTK's own
import table never actually references) but is simple, requires no extra tooling beyond what
this script bootstraps itself, and is still functionally correct: nothing is missing.

If you have MSYS2 installed anyway, prefer running fetch-windows-native-deps.sh from its UCRT64
bash shell for a tighter, import-verified bundle.

Prerequisites: PowerShell 5.1+, .NET SDK (dotnet) on PATH, and Windows' built-in tar.exe
(bundled since Windows 10 1803). zstd is bootstrapped automatically (downloaded from the
official zstd GitHub release) since Windows has no built-in zstd decompressor.
#>

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$RepoRoot = Split-Path -Parent $PSScriptRoot
$ProjectDir = Join-Path $RepoRoot "src\CSGtk.Gtk.Native.Windows"
$BundleDir = Join-Path $ProjectDir "runtimes\win-x64\native"
$WorkDir = Join-Path ([System.IO.Path]::GetTempPath()) ("csgtk-win-deps-" + [guid]::NewGuid())
New-Item -ItemType Directory -Path $WorkDir | Out-Null

$Msys2RepoUrl = "https://repo.msys2.org/mingw/ucrt64"
$Msys2Prefix = "mingw-w64-ucrt-x86_64-"
$RootPackage = "${Msys2Prefix}gtk4"
$RootDll = "libgtk-4-1.dll"

try {
    Write-Host "==> Fetching zstd (Windows has no built-in zstd decompressor)..."
    $zstdRelease = Invoke-RestMethod -Uri "https://api.github.com/repos/facebook/zstd/releases/latest"
    $zstdAsset = $zstdRelease.assets | Where-Object { $_.name -like "zstd-v*-win64.zip" } | Select-Object -First 1
    if (-not $zstdAsset) { throw "Could not find a win64 zstd release asset." }
    $zstdZip = Join-Path $WorkDir "zstd.zip"
    Invoke-WebRequest -Uri $zstdAsset.browser_download_url -OutFile $zstdZip
    $zstdToolDir = Join-Path $WorkDir "zstd-tool"
    Expand-Archive -Path $zstdZip -DestinationPath $zstdToolDir
    $zstdExe = Get-ChildItem -Path $zstdToolDir -Filter "zstd.exe" -Recurse | Select-Object -First 1 -ExpandProperty FullName
    if (-not $zstdExe) { throw "zstd.exe not found inside the downloaded archive." }

    Write-Host "==> Downloading MSYS2 UCRT64 package database..."
    $dbPath = Join-Path $WorkDir "ucrt64.db"
    Invoke-WebRequest -Uri "$Msys2RepoUrl/ucrt64.db" -OutFile $dbPath
    $dbTar = Join-Path $WorkDir "ucrt64.tar"
    & $zstdExe -d -f $dbPath -o $dbTar
    $dbDir = Join-Path $WorkDir "db"
    New-Item -ItemType Directory -Path $dbDir | Out-Null
    tar -xf $dbTar -C $dbDir

    Write-Host "==> Resolving dependency closure for $RootPackage..."
    $packages = @{}
    $provides = @{}
    Get-ChildItem -Path $dbDir -Directory | ForEach-Object {
        $descPath = Join-Path $_.FullName "desc"
        if (-not (Test-Path $descPath)) { return }
        $content = "`n" + (Get-Content -Path $descPath -Raw)
        $sections = [regex]::Split($content, "`n%([A-Z0-9_]+)%`n")
        $data = @{}
        for ($i = 1; $i -lt $sections.Length; $i += 2) {
            $key = $sections[$i]
            $rawVal = $sections[$i + 1].Trim("`n")
            $data[$key] = @($rawVal -split "`n" | Where-Object { $_ })
        }
        $name = ($data["NAME"] | Select-Object -First 1)
        if (-not $name) { return }
        $packages[$name] = @{
            FileName = ($data["FILENAME"] | Select-Object -First 1)
            Depends  = @($data["DEPENDS"] | ForEach-Object { ($_ -split '[<>=]')[0] })
        }
        foreach ($prov in $data["PROVIDES"]) {
            $provides[($prov -split '[<>=]')[0]] = $name
        }
    }

    function Resolve-Package([string]$name) {
        if ($packages.ContainsKey($name)) { return $name }
        if ($provides.ContainsKey($name)) { return $provides[$name] }
        return $null
    }

    $seen = New-Object System.Collections.Generic.HashSet[string]
    $missing = New-Object System.Collections.Generic.List[string]
    $queue = New-Object System.Collections.Generic.Queue[string]
    $queue.Enqueue($RootPackage)
    while ($queue.Count -gt 0) {
        $cur = $queue.Dequeue()
        $real = Resolve-Package $cur
        if (-not $real) {
            if ($cur.StartsWith($Msys2Prefix)) { [void]$missing.Add($cur) }
            continue
        }
        if ($seen.Contains($real)) { continue }
        [void]$seen.Add($real)
        foreach ($dep in $packages[$real].Depends) {
            $realDep = Resolve-Package $dep
            if (-not $realDep -or -not $seen.Contains($realDep)) { $queue.Enqueue($dep) }
        }
    }

    if ($missing.Count -gt 0) {
        throw "Could not resolve these packages in the MSYS2 repo:`n$($missing -join "`n")"
    }

    $filenames = @($seen | ForEach-Object { $packages[$_].FileName } | Where-Object { $_ })
    Write-Host "==> Downloading $($filenames.Count) candidate packages..."
    $downloadDir = Join-Path $WorkDir "download"
    New-Item -ItemType Directory -Path $downloadDir | Out-Null
    foreach ($filename in $filenames) {
        Invoke-WebRequest -Uri "$Msys2RepoUrl/$filename" -OutFile (Join-Path $downloadDir $filename)
    }

    Write-Host "==> Extracting packages..."
    $extractedDir = Join-Path $WorkDir "extracted"
    New-Item -ItemType Directory -Path $extractedDir | Out-Null
    Get-ChildItem -Path $downloadDir -Filter "*.pkg.tar.zst" | ForEach-Object {
        $tarPath = Join-Path $WorkDir ($_.BaseName + ".tar")
        & $zstdExe -d -f $_.FullName -o $tarPath
        tar -xf $tarPath -C $extractedDir
    }

    $binDir = Join-Path $extractedDir "ucrt64\bin"
    if (-not (Test-Path (Join-Path $binDir $RootDll))) {
        throw "$RootDll not found after extraction - MSYS2 package layout may have changed."
    }

    Write-Host "==> Installing bundle into $BundleDir..."
    if (Test-Path $BundleDir) { Remove-Item -Path $BundleDir -Recurse -Force }
    New-Item -ItemType Directory -Path $BundleDir -Force | Out-Null
    Get-ChildItem -Path $binDir -Filter "*.dll" | Copy-Item -Destination $BundleDir

    $fileCount = (Get-ChildItem -Path $BundleDir).Count
    Write-Host "    $fileCount DLLs bundled (unfiltered - see script header re: fetch-windows-native-deps.sh for a tighter bundle)"

    Write-Host "==> Packing NuGet package..."
    dotnet pack (Join-Path $ProjectDir "CSGtk.Gtk.Native.Windows.csproj") -c Release -o (Join-Path $RepoRoot "src\local-packages")
    if ($LASTEXITCODE -ne 0) { throw "dotnet pack failed with exit code $LASTEXITCODE" }

    Write-Host "==> Done."
}
finally {
    Remove-Item -Path $WorkDir -Recurse -Force -ErrorAction SilentlyContinue
}
