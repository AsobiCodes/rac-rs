# SPDX-License-Identifier: MIT
param([Parameter(Mandatory=$true)][string]$SdkDir)
$ErrorActionPreference='Stop'
$out=Join-Path $PSScriptRoot 'build'
& (Join-Path $PSScriptRoot 'build-helper.ps1') -SdkDir $SdkDir -OutDir $out
$stage=Join-Path $out 'package/rac-rtx-video'
New-Item -ItemType Directory -Path $stage -Force | Out-Null
$mod=Join-Path $PSScriptRoot 'mod'
foreach($name in @('omni-mod.json','main.luau','rtx-video.ini','LICENSE','README.md')){
    Copy-Item -LiteralPath (Join-Path $mod $name) -Destination $stage
}
foreach($name in @('rac-rtx-video-worker.exe','nvngx_vsr.dll','NVIDIA_RTX_Video_SDK_License.pdf')){
    Copy-Item -LiteralPath (Join-Path $out $name) -Destination $stage
}
$notice='The independent helper is MIT licensed (LICENSE). NVIDIA SDK/runtime components are governed by NVIDIA_RTX_Video_SDK_License.pdf. The companion game host integration is GPL-3.0-only. NVIDIA and RTX are NVIDIA trademarks. This is an unofficial community mod by Agers.'
[IO.File]::WriteAllText((Join-Path $stage 'THIRD_PARTY_LICENSES.txt'),$notice,[Text.UTF8Encoding]::new($false))
$version=(Get-Content -LiteralPath (Join-Path $mod 'omni-mod.json') -Raw | ConvertFrom-Json).version
$zip=Join-Path $out "rac-rtx-video-$version.zip"
Compress-Archive -LiteralPath $stage -DestinationPath $zip -Force
Write-Output "Player package: $zip (requires the companion host integration)."
