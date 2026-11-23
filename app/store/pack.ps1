# Packs the built app as an MSIX for the Microsoft Store, using the Windows
# SDK's own tools. The package is left unsigned: the Store signs it after
# review, so it can't be installed by double-clicking.
#
#   npx tauri build --no-bundle      (in app/)
#   .\store\pack.ps1
param(
    [string]$Exe = (Join-Path $PSScriptRoot "..\..\target\release\blind-passcode-generator.exe"),
    [string]$Out = (Join-Path $PSScriptRoot "..\..\target\release\bundle\msix\Blind-Passcode-Generator-store-x64.msix")
)
$ErrorActionPreference = "Stop"
# the SDK's tools take a relative path from somewhere else than PowerShell does
$Exe = (Resolve-Path $Exe).Path
$Out = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)

$tools = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\10.*\x64\makeappx.exe" -ErrorAction SilentlyContinue |
    Sort-Object { [version]$_.Directory.Parent.Name } | Select-Object -Last 1 | ForEach-Object { $_.Directory.FullName }
if (-not $tools) { throw "The Windows SDK isn't installed, so there is no makeappx.exe." }

# A package's version is four numbers, and the Store keeps the last for itself.
$workspace = Join-Path $PSScriptRoot "..\..\Cargo.toml"
$cargo = (cargo metadata --no-deps --format-version 1 --manifest-path $workspace | ConvertFrom-Json).packages |
    Where-Object name -eq "locker-app" | ForEach-Object version
$version = ($cargo -split "-")[0] + ".0"

$work = Join-Path ([IO.Path]::GetTempPath()) "blind-passcode-generator-msix-$PID"
$layout = Join-Path $work "layout"
New-Item -ItemType Directory -Force $layout | Out-Null
try {
    # under the name the manifest starts
    Copy-Item $Exe (Join-Path $layout "blind-passcode-generator.exe")
    Copy-Item (Join-Path $PSScriptRoot "Assets") $layout -Recurse
    (Get-Content (Join-Path $PSScriptRoot "AppxManifest.xml") -Raw).Replace('Version="0.0.0.0"', "Version=""$version""") |
        Set-Content (Join-Path $layout "AppxManifest.xml") -Encoding utf8

    # The index that lets Windows pick the right size of each image. Without
    # the packaging section taken out, it would be split across several files
    # that only make sense in a bundle of packages.
    $config = Join-Path $work "priconfig.xml"
    & "$tools\makepri.exe" createconfig /cf $config /dq en-US /o | Out-Null
    if ($LASTEXITCODE) { throw "makepri createconfig failed." }
    $xml = [xml](Get-Content $config -Raw)
    @($xml.resources.SelectNodes("packaging")) | ForEach-Object { [void]$xml.resources.RemoveChild($_) }
    $xml.Save($config)
    & "$tools\makepri.exe" new /pr $layout /cf $config /of (Join-Path $layout "resources.pri") /mn (Join-Path $layout "AppxManifest.xml") /o | Out-Null
    if ($LASTEXITCODE) { throw "makepri couldn't index the images." }

    New-Item -ItemType Directory -Force (Split-Path $Out) | Out-Null
    & "$tools\makeappx.exe" pack /d $layout /p $Out /o
    if ($LASTEXITCODE) { throw "makeappx couldn't build the package." }
}
finally {
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
}
"Packed Blind Passcode Generator $version for the Store: $((Resolve-Path $Out).Path)"
