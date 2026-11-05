@echo off
setlocal EnableExtensions
chcp 65001 >nul
cd /d "%~dp0"
if not exist "logs" mkdir "logs"
set "PPP_LOG=logs\launcher.log"

set "PPP_EXE="
if exist "bin\rot.exe" set "PPP_EXE=bin\rot.exe"
if not defined PPP_EXE if exist "rust\target\release\rot.exe" set "PPP_EXE=rust\target\release\rot.exe"
if not defined PPP_EXE goto :no_binary

"%PPP_EXE%" %*
set "PPP_RC=%ERRORLEVEL%"
echo [%DATE% %TIME%] %PPP_EXE% %* - exit %PPP_RC% >>"%PPP_LOG%"
if "%PPP_RC%"=="0" goto :done
if "%PPP_RC%"=="130" goto :done
echo.
echo [!] exit code %PPP_RC% - log: %PPP_LOG%
pause
goto :done

:no_binary
echo [!] rot.exe not found. Build it: cd rust ^&^& cargo build --release
echo [!] rot.exe not found.>>"%PPP_LOG%"
pause
exit /b 127

:done
endlocal & exit /b %PPP_RC%
