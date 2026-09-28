@echo off
chcp 65001 >nul
setlocal

for %%I in ("%~dp0..\..") do set "project_directory=%%~fI"
set "source_file=%project_directory%\AGENTS.md"

if not exist "%source_file%" (
    echo Error: source file not found: "%source_file%".
    pause
    exit /b 1
)

:prompt_location
echo Installation location:
echo   1. Default Codex directory: "%USERPROFILE%\.codex"
echo   2. Custom directory
set "location_choice="
set /p "location_choice=Select an installation location [1]: "
if not defined location_choice set "location_choice=1"
if "%location_choice%"=="1" goto use_default_directory
if "%location_choice%"=="2" goto prompt_directory
echo Invalid selection. Please try again.
goto prompt_location

:use_default_directory
set "target_directory=%USERPROFILE%\.codex"
goto directory_selected

:prompt_directory
set "target_directory="
set /p "target_directory=Enter the target directory: "
set "target_directory=%target_directory:"=%"

if not defined target_directory (
    echo The directory cannot be empty. Please try again.
    goto prompt_directory
)

:directory_selected
if not exist "%target_directory%\" goto confirm_create
goto check_target_file

:confirm_create
choice /c YN /n /m "The directory does not exist. Create it? [Y/N] "
if errorlevel 2 goto cancelled
mkdir "%target_directory%"
if errorlevel 1 goto failed_create

:check_target_file
if not exist "%target_directory%\AGENTS.md" goto copy_file
choice /c YN /n /m "AGENTS.md already exists in the target directory. Overwrite it? [Y/N] "
if errorlevel 2 goto cancelled

:copy_file
copy /Y "%source_file%" "%target_directory%\AGENTS.md" >nul
if errorlevel 1 goto failed_copy
echo AGENTS.md was copied to "%target_directory%".
pause
exit /b 0

:failed_create
echo Error: failed to create the directory "%target_directory%".
pause
exit /b 1

:failed_copy
echo Error: failed to copy AGENTS.md.
pause
exit /b 1

:cancelled
echo Operation cancelled.
pause
exit /b 0
