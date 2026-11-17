# Packages the Windows build: packaging/windows.ps1 <csdict.exe> <out.zip>
param([Parameter(Mandatory)] [string] $Exe, [Parameter(Mandatory)] [string] $Out)
$ErrorActionPreference = "Stop"
$dir = Join-Path ([System.IO.Path]::GetTempPath()) "csdict-package"
Remove-Item -Recurse -Force $dir -ErrorAction SilentlyContinue
New-Item -ItemType Directory $dir | Out-Null
Copy-Item $Exe (Join-Path $dir "csdict.exe")
Copy-Item README.md $dir
Compress-Archive -Path "$dir/*" -DestinationPath $Out -Force
