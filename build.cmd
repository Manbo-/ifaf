@echo off
setlocal

cargo build --release --target x86_64-pc-windows-msvc --offline
if errorlevel 1 exit /b %errorlevel%

if not exist "%~dp0dist" mkdir "%~dp0dist"
copy /Y "%~dp0target\x86_64-pc-windows-msvc\release\ifaf_susie.dll" "%~dp0dist\ifaf.sph" >nul
if errorlevel 1 exit /b %errorlevel%

echo Built "%~dp0dist\ifaf.sph"
