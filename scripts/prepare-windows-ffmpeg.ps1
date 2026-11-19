$ErrorActionPreference = "Stop"

$archiveUrl = "https://github.com/FFmpeg-Builds/FFmpeg-Builds/releases/download/autobuild-2026-11-14-12-37/ffmpeg-N-128909-g604ef3784d-win64-gpl.zip"
$expectedSha256 = "1adf262a8238d059a3a6e1949f026ba4b071bfa29b2399af53557cbc63008658"
$temporaryRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$archive = Join-Path $temporaryRoot "subaroll-ffmpeg.zip"
$expanded = Join-Path $temporaryRoot "subaroll-ffmpeg"
$destination = Join-Path $PSScriptRoot "..\src-tauri\binaries"

Invoke-WebRequest -Uri $archiveUrl -OutFile $archive
$actualSha256 = (Get-FileHash -Path $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha256 -ne $expectedSha256) {
  throw "FFmpeg archive checksum mismatch. Expected $expectedSha256, received $actualSha256."
}

if (Test-Path $expanded) { Remove-Item $expanded -Recurse -Force }
Expand-Archive -Path $archive -DestinationPath $expanded
New-Item -ItemType Directory -Path $destination -Force | Out-Null

$ffmpeg = Get-ChildItem -Path $expanded -Filter "ffmpeg.exe" -Recurse | Select-Object -First 1
$ffprobe = Get-ChildItem -Path $expanded -Filter "ffprobe.exe" -Recurse | Select-Object -First 1
if (-not $ffmpeg -or -not $ffprobe) { throw "FFmpeg or FFprobe was missing from the verified archive." }

Copy-Item $ffmpeg.FullName (Join-Path $destination "ffmpeg-x86_64-pc-windows-msvc.exe") -Force
Copy-Item $ffprobe.FullName (Join-Path $destination "ffprobe-x86_64-pc-windows-msvc.exe") -Force

$filters = & (Join-Path $destination "ffmpeg-x86_64-pc-windows-msvc.exe") -hide_banner -filters 2>&1 | Out-String
if ($LASTEXITCODE -ne 0 -or $filters -notmatch "subtitles\s+V->V") {
  throw "The bundled FFmpeg build does not expose the libass subtitles filter."
}

& (Join-Path $destination "ffprobe-x86_64-pc-windows-msvc.exe") -version
if ($LASTEXITCODE -ne 0) {
  throw "The bundled FFprobe executable could not run."
}
