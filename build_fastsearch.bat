@echo off
setlocal

cd /d "%~dp0"
set "RUSTFLAGS=%RUSTFLAGS% -C link-arg=/Brepro"

echo Building release fastsearch.exe...
cargo build --locked --release
if errorlevel 1 goto :build_error

copy /Y "target\release\fastsearch.exe" "fastsearch.exe" >nul
if errorlevel 1 goto :copy_error

echo.
echo.
echo Ready: %CD%\fastsearch.exe
exit /b 0

:build_error
echo.
echo Error: Rust project build failed.
exit /b 1

:copy_error
echo.
echo Error: could not create fastsearch.exe in the project root.
exit /b 1
