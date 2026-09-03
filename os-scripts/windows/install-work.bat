@echo off
chcp 65001 >nul
setlocal EnableExtensions EnableDelayedExpansion

if not "%~1"=="" goto invalid_arguments

for %%I in ("%~dp0..\..") do set "project_directory=%%~fI"
set "source_work=%project_directory%\skills\work"
set "missing_source="

call :validate_python_runtime
if errorlevel 1 goto runtime_error

call :validate_base_sources
if errorlevel 1 goto source_error

:prompt_home_choice
echo Installation location:
echo   1. Default user directory: "%USERPROFILE%"
echo   2. Custom user directory
set "home_choice="
set /p "home_choice=Select an installation location [1]: "
if not defined home_choice set "home_choice=1"
if "!home_choice!"=="1" (
    set "install_home=%USERPROFILE%"
    goto home_selected
)
if "!home_choice!"=="2" goto prompt_custom_home
echo Invalid selection. Please try again.
goto prompt_home_choice

:prompt_custom_home
set "install_home="
set /p "install_home=Enter the user directory: "
set "install_home=!install_home:"=!"
if not defined install_home (
    echo The directory cannot be empty. Please try again.
    goto prompt_custom_home
)

:home_selected
if not defined install_home (
    echo Error: the selected user directory is unavailable.
    pause
    exit /b 1
)
for %%I in ("!install_home!") do set "install_home=%%~fI"
if not exist "!install_home!\." (
    echo The user directory does not exist: "!install_home!".
    if "!home_choice!"=="2" goto prompt_custom_home
    pause
    exit /b 1
)
set "target_work=!install_home!\.agents\skills\work"

:prompt_hierarchy
echo Instruction hierarchy:
echo   1. general only
echo   2. web
echo   3. backend
echo   4. java
echo   5. jpa
echo   6. mybatis
echo   7. frontend
echo   8. typescript
echo   9. astro
echo   10. css
echo   11. tailwind
echo Select multiple branches with spaces. Parent branches are included automatically.
set "hierarchy_selection="
set /p "hierarchy_selection=Select hierarchy numbers, enter "all", or press Enter for general only: "
if not defined hierarchy_selection set "hierarchy_selection=1"

set "include_web="
set "include_backend="
set "include_java="
set "include_jpa="
set "include_mybatis="
set "include_frontend="
set "include_typescript="
set "include_astro="
set "include_css="
set "include_tailwind="

if /i "!hierarchy_selection!"=="all" (
    call :include_hierarchy 5
    call :include_hierarchy 6
    call :include_hierarchy 9
    call :include_hierarchy 11
    goto hierarchy_selected
)

for /f "delims=0123456789 " %%A in ("!hierarchy_selection!") do goto invalid_hierarchy
for %%N in (!hierarchy_selection!) do (
    if %%N lss 1 goto invalid_hierarchy
    if %%N gtr 11 goto invalid_hierarchy
)
for %%N in (!hierarchy_selection!) do call :include_hierarchy %%N

:hierarchy_selected
call :validate_selected_instructions
if errorlevel 1 goto source_error

if not exist "!target_work!\" (
    mkdir "!target_work!"
    if errorlevel 1 goto install_error
)

call :install_base
if errorlevel 1 goto install_error
call :install_selected_instructions
if errorlevel 1 goto install_error

echo Work skill installed in "!target_work!".
echo Existing matching files were overwritten. Stale files were not removed.
pause
exit /b 0

:validate_python_runtime
set "python_command="
py -3 -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)" <nul >nul 2>nul
if not errorlevel 1 set "python_command=py -3"
if defined python_command (
    call :validate_pyyaml
    exit /b !errorlevel!
)
python -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)" <nul >nul 2>nul
if not errorlevel 1 set "python_command=python"
if defined python_command (
    call :validate_pyyaml
    exit /b !errorlevel!
)
python3 -c "import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)" <nul >nul 2>nul
if not errorlevel 1 set "python_command=python3"
if defined python_command (
    call :validate_pyyaml
    exit /b !errorlevel!
)
set "runtime_error_message=Python 3.10 or newer is required. Install Python and run this installer again."
exit /b 1

:validate_pyyaml
!python_command! -c "import yaml" <nul >nul 2>nul
if errorlevel 1 (
    set "runtime_error_message=PyYAML is required. Install PyYAML for !python_command! and run this installer again."
    exit /b 1
)
exit /b 0

:validate_base_sources
call :require_file "SKILL.md"
if errorlevel 1 exit /b 1
call :require_file "agents\openai.yaml"
if errorlevel 1 exit /b 1
call :require_file "references\instruction-loading.md"
if errorlevel 1 exit /b 1
for %%M in (plan task execute) do (
    call :require_file "references\workflows\%%M.md"
    if errorlevel 1 exit /b 1
    call :require_file "references\subagents\%%M.md"
    if errorlevel 1 exit /b 1
)
call :require_file "scripts\work.py"
if errorlevel 1 exit /b 1
call :require_file "scripts\worklib\cli.py"
if errorlevel 1 exit /b 1
exit /b 0

:validate_selected_instructions
for %%M in (plan task execute) do (
    call :require_instruction "%%M" "general"
    if errorlevel 1 exit /b 1
)
if defined include_web for %%M in (plan task execute) do (
    call :require_instruction "%%M" "web"
    if errorlevel 1 exit /b 1
)
if defined include_backend for %%M in (plan task execute) do (
    call :require_instruction "%%M" "web\backend"
    if errorlevel 1 exit /b 1
)
if defined include_java for %%M in (plan task execute) do (
    call :require_instruction "%%M" "web\backend\java"
    if errorlevel 1 exit /b 1
)
if defined include_jpa for %%M in (task execute) do (
    call :require_instruction "%%M" "web\backend\java\jpa"
    if errorlevel 1 exit /b 1
)
if defined include_mybatis for %%M in (task execute) do (
    call :require_instruction "%%M" "web\backend\java\mybatis"
    if errorlevel 1 exit /b 1
)
if defined include_frontend for %%M in (plan task execute) do (
    call :require_instruction "%%M" "web\frontend"
    if errorlevel 1 exit /b 1
)
if defined include_typescript for %%M in (plan task execute) do (
    call :require_instruction "%%M" "web\frontend\typescript"
    if errorlevel 1 exit /b 1
)
if defined include_astro for %%M in (task execute) do (
    call :require_instruction "%%M" "web\frontend\typescript\astro"
    if errorlevel 1 exit /b 1
)
if defined include_css for %%M in (plan task execute) do (
    call :require_instruction "%%M" "web\frontend\css"
    if errorlevel 1 exit /b 1
)
if defined include_tailwind for %%M in (task execute) do (
    call :require_instruction "%%M" "web\frontend\css\tailwind"
    if errorlevel 1 exit /b 1
)
exit /b 0

:require_file
if exist "!source_work!\%~1" exit /b 0
set "missing_source=!source_work!\%~1"
exit /b 1

:require_instruction
call :require_file "references\instructions\%~1\%~2\instructions.md"
exit /b %errorlevel%

:include_hierarchy
if "%~1"=="2" set "include_web=1"
if "%~1"=="3" (
    set "include_web=1"
    set "include_backend=1"
)
if "%~1"=="4" (
    set "include_web=1"
    set "include_backend=1"
    set "include_java=1"
)
if "%~1"=="5" (
    set "include_web=1"
    set "include_backend=1"
    set "include_java=1"
    set "include_jpa=1"
)
if "%~1"=="6" (
    set "include_web=1"
    set "include_backend=1"
    set "include_java=1"
    set "include_mybatis=1"
)
if "%~1"=="7" (
    set "include_web=1"
    set "include_frontend=1"
)
if "%~1"=="8" (
    set "include_web=1"
    set "include_frontend=1"
    set "include_typescript=1"
)
if "%~1"=="9" (
    set "include_web=1"
    set "include_frontend=1"
    set "include_typescript=1"
    set "include_astro=1"
)
if "%~1"=="10" (
    set "include_web=1"
    set "include_frontend=1"
    set "include_css=1"
)
if "%~1"=="11" (
    set "include_web=1"
    set "include_frontend=1"
    set "include_css=1"
    set "include_tailwind=1"
)
exit /b 0

:install_base
call :copy_file "SKILL.md"
if errorlevel 1 exit /b 1
call :copy_file "agents\openai.yaml"
if errorlevel 1 exit /b 1
call :copy_file "references\instruction-loading.md"
if errorlevel 1 exit /b 1
call :copy_tree "references\workflows"
if errorlevel 1 exit /b 1
call :copy_tree "references\subagents"
if errorlevel 1 exit /b 1
call :copy_file "scripts\work.py"
if errorlevel 1 exit /b 1
for /r "%source_work%\scripts\worklib" %%F in (*.py) do (
    if /i not "%%~nxF"=="rules.py" (
        set "python_relative=%%~fF"
        set "python_relative=!python_relative:%source_work%\=!"
        call :copy_file "!python_relative!"
        if errorlevel 1 exit /b 1
    )
)
exit /b 0

:install_selected_instructions
for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "general"
    if errorlevel 1 exit /b 1
)
if defined include_web for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "web"
    if errorlevel 1 exit /b 1
)
if defined include_backend for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "web\backend"
    if errorlevel 1 exit /b 1
)
if defined include_java for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "web\backend\java"
    if errorlevel 1 exit /b 1
)
if defined include_jpa for %%M in (task execute) do (
    call :copy_instruction "%%M" "web\backend\java\jpa"
    if errorlevel 1 exit /b 1
)
if defined include_mybatis for %%M in (task execute) do (
    call :copy_instruction "%%M" "web\backend\java\mybatis"
    if errorlevel 1 exit /b 1
)
if defined include_frontend for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "web\frontend"
    if errorlevel 1 exit /b 1
)
if defined include_typescript for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "web\frontend\typescript"
    if errorlevel 1 exit /b 1
)
if defined include_astro for %%M in (task execute) do (
    call :copy_instruction "%%M" "web\frontend\typescript\astro"
    if errorlevel 1 exit /b 1
)
if defined include_css for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "web\frontend\css"
    if errorlevel 1 exit /b 1
)
if defined include_tailwind for %%M in (task execute) do (
    call :copy_instruction "%%M" "web\frontend\css\tailwind"
    if errorlevel 1 exit /b 1
)
exit /b 0

:copy_instruction
set "instruction_relative=references\instructions\%~1\%~2"
call :copy_file "!instruction_relative!\instructions.md"
if errorlevel 1 exit /b 1
if exist "!source_work!\!instruction_relative!\references\" (
    call :copy_tree "!instruction_relative!\references"
    if errorlevel 1 exit /b 1
)
exit /b 0

:copy_file
set "copy_relative=%~1"
set "copy_source=!source_work!\!copy_relative!"
set "copy_target=!target_work!\!copy_relative!"
for %%D in ("!copy_target!\..") do set "copy_parent=%%~fD"
if not exist "!copy_parent!\" (
    mkdir "!copy_parent!"
    if errorlevel 1 exit /b 1
)
copy /Y "!copy_source!" "!copy_target!" >nul
if errorlevel 1 exit /b 1
exit /b 0

:copy_tree
set "tree_relative=%~1"
set "tree_source=!source_work!\!tree_relative!"
set "tree_target=!target_work!\!tree_relative!"
if not exist "!tree_target!\" (
    mkdir "!tree_target!"
    if errorlevel 1 exit /b 1
)
xcopy "!tree_source!\*" "!tree_target!\" /E /I /H /R /Y /Q >nul
if errorlevel 1 exit /b 1
exit /b 0

:invalid_hierarchy
echo Invalid selection. Please try again.
goto prompt_hierarchy

:invalid_arguments
echo Error: this installer no longer accepts plan, task, execute, or all arguments.
exit /b 2

:runtime_error
echo Error: !runtime_error_message!
echo The installer does not install Python packages automatically.
pause
exit /b 1

:source_error
echo Error: required Work skill source not found: "!missing_source!".
pause
exit /b 1

:install_error
echo Error: failed to install the Work skill in "!target_work!".
pause
exit /b 1
