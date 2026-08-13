@echo off
setlocal

cd /d "%~dp0"
set "FASTSEARCH_TARGET_DIR=%~dp0..\.cargo-target\FastSearch"

echo Building release fastsearch.exe...
cargo build --locked --release --target-dir "%FASTSEARCH_TARGET_DIR%"
if errorlevel 1 goto :build_error

copy /Y "%FASTSEARCH_TARGET_DIR%\release\fastsearch.exe" "fastsearch.exe" >nul
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
