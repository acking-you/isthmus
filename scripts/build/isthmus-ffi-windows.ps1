# Build the Windows FFI DLL and stage it for Flutter.
# - Copies it to ui/native/windows/x64, which windows/CMakeLists.txt installs
#   beside Isthmus.exe
# - Copies it into existing build output folders, so `flutter run` picks up
#   a rebuilt library without a clean

$ErrorActionPreference = "Stop"

$root = (Resolve-Path (Join-Path $PSScriptRoot "../..") | Select-Object -First 1 -ExpandProperty Path)
$ffiLib = "isthmus_ffi.dll"
# Not $profile: PowerShell already names the profile script that.
$buildProfile = if ($env:PROFILE) { $env:PROFILE } else { "release" }
# Built up with +=: an `if` that yields a one-element array hands back the
# bare string, and splatting a string passes its characters one by one.
$cargoArgs = @("build", "-p", "isthmus-ffi")
if ($buildProfile -eq "release") { $cargoArgs += "--release" }

Push-Location $root
try {
  & cargo @cargoArgs
  if ($LASTEXITCODE -ne 0) { throw "FFI build failed" }

  $built = Join-Path $root "target/$buildProfile/$ffiLib"
  $nativeDir = Join-Path $root "ui/native/windows/x64"
  New-Item -ItemType Directory -Force $nativeDir | Out-Null
  Copy-Item $built (Join-Path $nativeDir $ffiLib) -Force

  $candidateDirs = @(
    (Join-Path -Path $root -ChildPath "ui/build/windows/x64/runner/Release");
    (Join-Path -Path $root -ChildPath "ui/build/windows/x64/runner/Debug");
    (Join-Path -Path $root -ChildPath "ui/build/windows/x64/runner/Profile")
  )
  foreach ($dir in $candidateDirs) {
    if (Test-Path $dir) {
      Copy-Item $built (Join-Path $dir $ffiLib) -Force
    }
  }

  Write-Host "FFI ready: $nativeDir\$ffiLib"
} finally {
  Pop-Location
}
