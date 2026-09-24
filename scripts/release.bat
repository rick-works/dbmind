@echo off
rem ---------------------------------------------------------------------------
rem One-click release build: portable bundle (+ zip) and desktop installers.
rem
rem   Double-click it, or right-click -> "Run". No admin rights required.
rem   -ExecutionPolicy Bypass is per-invocation: nothing on this machine changes.
rem   -NoProfile keeps a user profile from interfering.
rem   Defaults if you pass NO arguments: -WithJre -Jdk D:\develop\tools\jdk-25.0.4.1+1
rem   i.e. double-clicking builds a package that carries its own Java runtime,
rem   generated from that JDK. Pass any argument to override the defaults, e.g.:
rem       release.bat -SkipPortable                      only build installers
rem       release.bat -SkipBundle                        only build the portable bundle
rem       release.bat -WithJre -Jdk "D:\some\jdk-21"      pin a different JDK
rem       release.bat -SkipBuild -SkipPortable           reuse existing build, installers only
rem
rem ASCII only: cmd.exe reads .bat in the OEM code page, so non-ASCII text here
rem would show up garbled. The PowerShell script carries the Chinese output, and
rem chcp 65001 makes that come out right.
rem ---------------------------------------------------------------------------
chcp 65001 >nul
setlocal
set "SCRIPT=%~dp0release.ps1"
if not exist "%SCRIPT%" (
    echo [ERROR] not found: %SCRIPT%
    echo         release.bat must sit next to release.ps1 ^(in scripts\^).
    echo.
    pause
    exit /b 1
)
if "%~1"=="" (
    rem No arguments: use the project's chosen defaults (bundled JRE from this JDK)
    powershell -NoProfile -ExecutionPolicy Bypass -File "%SCRIPT%" -WithJre -Jdk D:\develop\tools\jdk-25.0.4.1+1
) else (
    powershell -NoProfile -ExecutionPolicy Bypass -File "%SCRIPT%" %*
)
set "CODE=%ERRORLEVEL%"
echo.
if not "%CODE%"=="0" (
    echo [FAILED] exit code %CODE% - see the output above.
) else (
    echo [DONE] exit code 0.
)
echo.
pause
exit /b %CODE%
