@echo off
setlocal
rem Keep this launcher beside rztop.exe. Focus mode hides the title bar and tabs.
if not exist "%~dp0rztop.exe" (
    echo rztop.exe was not found beside this launcher.
    pause
    exit /b 1
)
set "rztop_terminal=%LOCALAPPDATA%\Microsoft\WindowsApps\wt.exe"
if not exist "%rztop_terminal%" (
    set "rztop_terminal=wt.exe"
    where.exe wt.exe >nul 2>nul
    if errorlevel 1 (
        echo Windows Terminal was not found. Install it or enable its wt.exe app execution alias.
        pause
        exit /b 1
    )
)
start "" "%rztop_terminal%" --window new --focus new-tab --title rztop --startingDirectory "%~dp0." "%~dp0rztop.exe" %*
exit /b %errorlevel%
