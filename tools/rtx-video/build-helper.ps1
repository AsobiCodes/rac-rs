# SPDX-License-Identifier: MIT
param([Parameter(Mandatory=$true)][string]$SdkDir,[string]$OutDir=(Join-Path $PSScriptRoot 'build'))
$ErrorActionPreference='Stop'
$sdkPath=(Resolve-Path -LiteralPath $SdkDir).Path
New-Item -ItemType Directory -Path $OutDir -Force | Out-Null
$outputPath=(Resolve-Path -LiteralPath $OutDir).Path
$sourcePath=Join-Path $PSScriptRoot 'src/rtx_video_worker.cpp'
$vswherePath='C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
$vsPath=& $vswherePath -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if(-not $vsPath){throw 'Visual Studio C++ Build Tools are required to build the helper.'}
$vcvarsPath=Join-Path $vsPath 'VC/Auxiliary/Build/vcvars64.bat'
foreach($pathValue in @($sdkPath,$outputPath,$sourcePath,$vcvarsPath)){if($pathValue -match '[%\r\n]'){throw 'Build paths must not contain percent or newline characters.'}}
$batchPath=Join-Path $outputPath 'build-helper.cmd'
$lines=@('@echo off',('call "'+$vcvarsPath+'" >nul'),('cl /nologo /O2 /EHsc /std:c++17 /MT /DNDEBUG /I"'+$sdkPath+'\include" /Fo"'+$outputPath+'\rtx_video_worker.obj" "'+$sourcePath+'" /link /LIBPATH:"'+$sdkPath+'\lib\Windows\x64" nvsdk_ngx_s.lib d3d11.lib dxgi.lib advapi32.lib wintrust.lib crypt32.lib version.lib bcrypt.lib shlwapi.lib user32.lib ole32.lib shell32.lib /OUT:"'+$outputPath+'\rac-rtx-video-worker.exe"'))
[IO.File]::WriteAllLines($batchPath,$lines,[Text.Encoding]::ASCII)
& $env:ComSpec /d /c $batchPath
if($LASTEXITCODE -ne 0){throw 'RTX Video helper build failed.'}
Copy-Item -LiteralPath (Join-Path $sdkPath 'bin/Windows/x64/rel/nvngx_vsr.dll') -Destination $outputPath
Copy-Item -LiteralPath (Join-Path $sdkPath 'NVIDIA_RTX_Video_SDK_License.pdf') -Destination $outputPath
