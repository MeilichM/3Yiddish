@echo off
rem Run the desktop app in dev mode against a test home, with WebView2's debug port open.
rem   launch.cmd [test-home] [gpu]
rem "gpu" builds with Vulkan (needs the Vulkan SDK; VULKAN_SDK set) into the short folder C:\t3y,
rem because the Vulkan shader build passes Windows' 260-character path limit otherwise.
setlocal
set "TESTHOME=%~1"
if "%TESTHOME%"=="" set "TESTHOME=%TEMP%\3y-desktop-test"

for /f "usebackq delims=" %%i in (`"%ProgramFiles(x86)%\Microsoft Visual Studio\Installer\vswhere.exe" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath`) do set "VS=%%i"
if not defined VS (echo Visual Studio with the C++ tools is not installed. & exit /b 1)
call "%VS%\VC\Auxiliary\Build\vcvars64.bat" >nul || exit /b 1
set "LIBCLANG_PATH=%VS%\VC\Tools\Llvm\x64\bin"
set "PATH=%VS%\Common7\IDE\CommonExtensions\Microsoft\CMake\CMake\bin;%PATH%"

set "THREE_YIDDISH_HOME=%TESTHOME%"
rem A separate WebView profile: the real app's saved history (localStorage) stays untouched.
set "WEBVIEW2_USER_DATA_FOLDER=%TESTHOME%\webview"
set "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333"

cd /d "%~dp0..\..\..\..\desktop" || exit /b 1
if not exist node_modules call npm install || exit /b 1
if /i "%~2"=="gpu" (
  set "CARGO_TARGET_DIR=C:\t3y"
  npx tauri dev --features vulkan
) else (
  npx tauri dev
)
