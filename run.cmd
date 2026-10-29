@echo off
setlocal EnableExtensions
chcp 65001 >nul
cd /d "%~dp0"
if not exist "logs" mkdir "logs"
set "PPP_LOG=logs\launcher.log"

set "PPP_EXE="
set "PPP_ARGS="
if defined PPP_PY_CMD call :probe "%PPP_PY_CMD%"
if not defined PPP_EXE call :probe "%LOCALAPPDATA%\Programs\Python\Launcher\py.exe" -3
if not defined PPP_EXE call :probe "python"
if not defined PPP_EXE call :probe "python3"
if not defined PPP_EXE goto :no_python

"%PPP_EXE%" %PPP_ARGS% rot.py %*
set "PPP_RC=%ERRORLEVEL%"
echo [%DATE% %TIME%] "%PPP_EXE%" %PPP_ARGS% rot.py %* - exit %PPP_RC% >>"%PPP_LOG%"
if "%PPP_RC%"=="0" goto :done
if "%PPP_RC%"=="130" goto :done
echo.
echo [!] exit code %PPP_RC% - log: %PPP_LOG%
pause
goto :done

:no_python
echo [!] No working Python 3.8+ found.
echo     Checked PPP_PY_CMD, "%LOCALAPPDATA%\Programs\Python\Launcher\py.exe -3", python, python3.
echo     Point it at a real interpreter, for example:
echo       set PPP_PY_CMD=C:\path\to\venv\Scripts\python.exe
echo.
echo [!] No working Python 3.8+ found.>>"%PPP_LOG%"
pause
exit /b 127

:done
endlocal & exit /b %PPP_RC%

:probe
rem First candidate that exists, is not a WindowsApps stub, and runs Python 3.8+.
if defined PPP_EXE goto :eof
set "PPP_CAND=%~1"
set "PPP_CAND_ARGS=%~2"
if "%PPP_CAND%"=="" goto :eof
echo %PPP_CAND% | findstr /i /c:"\WindowsApps\" >nul && goto :eof
"%PPP_CAND%" %PPP_CAND_ARGS% -c "import sys; sys.exit(0 if sys.version_info >= (3, 8) else 1)" >nul 2>nul
if errorlevel 1 goto :eof
set "PPP_EXE=%PPP_CAND%"
set "PPP_ARGS=%PPP_CAND_ARGS%"
goto :eof
