@echo off
chcp 65001 >nul
setlocal

set "codex_directory=%USERPROFILE%\.codex"

echo Codex directory: "%codex_directory%"
echo This will permanently delete:
echo   sessions
echo   archived_sessions
echo   generated_images
echo   session_index.jsonl
echo   history.jsonl
echo   state_5.sqlite
echo   state_5.sqlite-shm
echo   state_5.sqlite-wal
echo Close Codex before continuing.

if not exist "%codex_directory%\" (
    echo No Codex directory was found. Nothing to clean.
    pause
    exit /b 0
)

choice /c YN /n /m "Permanently delete all local Codex data listed above? [Y/N] "
if errorlevel 2 (
    echo Operation cancelled.
    pause
    exit /b 0
)

set "cleanup_failed=0"

rmdir /s /q "%codex_directory%\sessions" 2>nul
if exist "%codex_directory%\sessions\" set "cleanup_failed=1"
rmdir /s /q "%codex_directory%\archived_sessions" 2>nul
if exist "%codex_directory%\archived_sessions\" set "cleanup_failed=1"
rmdir /s /q "%codex_directory%\generated_images" 2>nul
if exist "%codex_directory%\generated_images\" set "cleanup_failed=1"

del /f /q "%codex_directory%\session_index.jsonl" 2>nul
if exist "%codex_directory%\session_index.jsonl" set "cleanup_failed=1"
del /f /q "%codex_directory%\history.jsonl" 2>nul
if exist "%codex_directory%\history.jsonl" set "cleanup_failed=1"
del /f /q "%codex_directory%\state_5.sqlite" 2>nul
if exist "%codex_directory%\state_5.sqlite" set "cleanup_failed=1"
del /f /q "%codex_directory%\state_5.sqlite-shm" 2>nul
if exist "%codex_directory%\state_5.sqlite-shm" set "cleanup_failed=1"
del /f /q "%codex_directory%\state_5.sqlite-wal" 2>nul
if exist "%codex_directory%\state_5.sqlite-wal" set "cleanup_failed=1"

if "%cleanup_failed%"=="1" (
    echo Error: one or more local Codex data files could not be removed.
    pause
    exit /b 1
)

echo Local Codex data was cleaned successfully.
pause
exit /b 0
