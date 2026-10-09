# Makes the installer and the portable archive of MigPad for Windows in target\bundle from a
# release build of migpad.exe, on Windows with Inno Setup 6 and 7-Zip:
#
#     script\bundle-windows.ps1 -Arch x64 [-Exe target\release\migpad.exe]
#
# The licenses of the components, -Licenses (target\THIRD-PARTY-LICENSES.txt by default), go with
# them if there is such a file. The portable archive has an empty folder .migpad next to
# migpad.exe: MigPad keeps its data there.
param(
    [Parameter(Mandatory)][ValidateSet('x64', 'arm64')][string]$Arch,
    [string]$Exe = 'target\release\migpad.exe',
    [string]$Licenses = 'target\THIRD-PARTY-LICENSES.txt'
)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot)

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.*)"$' | Select-Object -First 1).Matches.Groups[1].Value
$bundle = 'target\bundle'
$staging = "$bundle\windows-$Arch"
Remove-Item -Recurse -Force $staging -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$staging\MigPad\.migpad" | Out-Null
Copy-Item $Exe "$staging\MigPad\migpad.exe"
Copy-Item LICENSE "$staging\MigPad\LICENSE.txt"
if (Test-Path $Licenses) {
    Copy-Item $Licenses "$staging\MigPad\THIRD-PARTY-LICENSES.txt"
}

$iscc = "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe"
& $iscc /Qp "/DVersion=$version" "/DArch=$Arch" "/DSource=$(Resolve-Path "$staging\MigPad")" "/DOutput=$(Resolve-Path $bundle)" crates\migpad\resources\windows\migpad.iss
if ($LASTEXITCODE -ne 0) { throw "Inno Setup failed: $LASTEXITCODE" }

# 7-Zip keeps the empty folder, which Compress-Archive leaves out.
$zip = "$bundle\MigPad-$Arch-portable.zip"
Remove-Item -Force $zip -ErrorAction SilentlyContinue
Push-Location $staging
& 7z a -tzip -bso0 "..\MigPad-$Arch-portable.zip" MigPad
$code = $LASTEXITCODE
Pop-Location
if ($code -ne 0) { throw "7-Zip failed: $code" }
Get-Item "$bundle\MigPad-$Arch-setup.exe", $zip | ForEach-Object { $_.FullName }
