# Run a command with the Windows build tools 3Yiddish needs on PATH:
#   powershell -File desktop\scripts\win-env.ps1 npm run tauri dev
# Finds a Visual Studio install that has the x64 C++ libraries (msvcrt.lib), the CMake that
# ships with it, and libclang (LLVM, or `pip install --target %USERPROFILE%\.libclang libclang`).
$ErrorActionPreference = 'Stop'
$vswhere = "${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -all -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath |
  Where-Object { Get-ChildItem "$_\VC\Tools\MSVC\*\lib\x64\msvcrt.lib" -ErrorAction SilentlyContinue } |
  Select-Object -First 1
if (-not $vs) { throw 'No Visual Studio with the C++ x64 libraries. Install "Desktop development with C++".' }

$extra = @("$env:USERPROFILE\.cargo\bin")
# CMake 3.21+ (the GPU build's shader step needs it): one on PATH, else a per-user one
# (`pip install --target %USERPROFILE%\.cmake cmake`), else the one Visual Studio ships.
if (-not (Get-Command cmake -ErrorAction SilentlyContinue)) {
  $cmake = @("$env:USERPROFILE\.cmake\cmake\data\bin", "$vs\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin") |
    Where-Object { Test-Path "$_\cmake.exe" } | Select-Object -First 1
  if ($cmake) { $extra += $cmake }
}
# Ninja builds with the compiler vcvars puts on PATH, so CMake cannot wander off to another
# Visual Studio (left alone it takes the newest, which may have no C++). The GPU build's
# shader tool is a sub-project, and it inherits Ninja too. `pip install --target
# %USERPROFILE%\.ninja ninja`, or the copy Visual Studio ships.
$ninja = @("$env:USERPROFILE\.ninja\bin", "$vs\Common7\IDE\CommonExtensions\Microsoft\CMake\Ninja") |
  Where-Object { Test-Path "$_\ninja.exe" } | Select-Object -First 1
if ((Get-Command ninja -ErrorAction SilentlyContinue) -or $ninja) {
  if ($ninja) { $extra += $ninja }
  $env:CMAKE_GENERATOR = 'Ninja'
}
# CMake 4 refuses projects that declare support for very old versions; whisper.cpp is fine.
$env:CMAKE_POLICY_VERSION_MINIMUM = '3.5'
if (-not $env:LIBCLANG_PATH) {
  $clang = @("$env:ProgramFiles\LLVM\bin", "$vs\VC\Tools\Llvm\x64\bin", "$env:USERPROFILE\.libclang\clang\native") |
    Where-Object { Test-Path "$_\libclang.dll" } | Select-Object -First 1
  if (-not $clang) { throw 'libclang not found. Install LLVM, or: pip install --target %USERPROFILE%\.libclang libclang' }
  $env:LIBCLANG_PATH = $clang
}
# The graphics-card (Vulkan) build needs the Vulkan SDK; a fresh install is not in this
# shell's environment yet, so look it up.
if (-not $env:VULKAN_SDK) {
  $sdk = [Environment]::GetEnvironmentVariable('VULKAN_SDK', 'Machine')
  if (-not $sdk) { $sdk = Get-ChildItem 'C:\VulkanSDK' -Directory -ErrorAction SilentlyContinue | Sort-Object Name | Select-Object -Last 1 -ExpandProperty FullName }
  if ($sdk) { $env:VULKAN_SDK = $sdk; $extra += "$sdk\Bin" }
}
# whisper.cpp's GPU build nests its shader tool ~190 characters deep, and the compiler cannot
# open files past 260 characters, so build output goes to a short folder instead of next to
# the sources (a checkout under Documents is already long).
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = "$env:USERPROFILE\.3yb" }
$command = $args -join ' '
cmd /c "`"$vs\VC\Auxiliary\Build\vcvars64.bat`" >nul 2>nul && set `"PATH=$($extra -join ';');%PATH%`" && $command"
exit $LASTEXITCODE
