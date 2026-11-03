@echo off
chcp 65001 >nul
setlocal
set "PATH=%USERPROFILE%\.mingw64\bin;%USERPROFILE%\.cargo\bin;%PATH%"
cd /d "%~dp0"
if exist "target\release\disk-app.exe" (
    start "" "target\release\disk-app.exe"
    exit /b 0
)
echo 未找到已构建的程序，正在编译（首次需要几分钟）...
cargo build --release -p disk-app
if errorlevel 1 (
    echo.
    echo 编译失败。请确认已安装 Rust 与 MinGW，并检查上方错误信息。
    pause
    exit /b 1
)
start "" "target\release\disk-app.exe"
