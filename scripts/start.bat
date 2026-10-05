@echo off
rem DBmind launcher (portable bundle).
rem ASCII only on purpose: cmd.exe reads .bat using the ANSI code page,
rem so non-ASCII comments would turn into garbage on some machines.

cd /d "%~dp0"

rem Already running? Just open the browser.
netstat -ano | findstr /r /c:"LISTENING.*:8787" >nul 2>&1
if %errorlevel%==0 goto :open

rem Start the shell (it serves both the API and the UI), then open the browser.
start "DBmind" /min "dbmind-web.exe" --port 8787 --dist web
timeout /t 3 /nobreak >nul

:open
start "" "http://127.0.0.1:8787/"
exit /b 0
