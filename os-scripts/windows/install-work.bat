@echo off
chcp 65001 <nul >nul
setlocal EnableExtensions EnableDelayedExpansion

if not "%~1"=="" goto invalid_arguments

for %%I in ("%~dp0..\..") do set "project_directory=%%~fI"
set "source_work=%project_directory%\skills\work"
set "missing_source="

call :validate_rust_toolchain
if errorlevel 1 goto runtime_error

call :validate_base_sources
if errorlevel 1 goto source_error

:prompt_home_choice
echo Installation location:
echo   1. Default installation directory: "%USERPROFILE%\.agents"
echo   2. Custom installation directory
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
set /p "install_home=Enter the installation directory: "
set "install_home=!install_home:"=!"
if not defined install_home (
    echo The directory cannot be empty. Please try again.
    goto prompt_custom_home
)

:home_selected
if not defined install_home (
    echo Error: the selected installation directory is unavailable.
    pause
    exit /b 1
)
for %%I in ("!install_home!") do set "install_home=%%~fI"
if not exist "!install_home!\." (
    echo The installation directory does not exist: "!install_home!".
    if "!home_choice!"=="2" goto prompt_custom_home
    pause
    exit /b 1
)
if "!home_choice!"=="1" set "install_home=!install_home!\.agents"
set "target_work=!install_home!\skills\work"

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
echo   12. spring-boot
echo Select multiple branches with spaces. Parent branches are included automatically.
echo Previously installed branches and stale files will be kept, even with general only.
set "hierarchy_selection="
set /p "hierarchy_selection=Select hierarchy numbers, enter "all", or press Enter for general only: "
if not defined hierarchy_selection set "hierarchy_selection=1"

set "include_web="
set "include_backend="
set "include_java="
set "include_spring="
set "include_persistence="
set "include_jpa="
set "include_mybatis="
set "include_frontend="
set "include_typescript="
set "include_astro="
set "include_css="
set "include_tailwind="

if /i "!hierarchy_selection!"=="all" (
    call :include_hierarchy 3
    call :include_hierarchy 5
    call :include_hierarchy 6
    call :include_hierarchy 8
    call :include_hierarchy 9
    call :include_hierarchy 11
    call :include_hierarchy 12
    goto hierarchy_selected
)

for /f "delims=0123456789 " %%A in ("!hierarchy_selection!") do goto invalid_hierarchy
for %%N in (!hierarchy_selection!) do (
    if %%N lss 1 goto invalid_hierarchy
    if %%N gtr 12 goto invalid_hierarchy
)
for %%N in (!hierarchy_selection!) do call :include_hierarchy %%N

:hierarchy_selected
call :validate_selected_instructions
if errorlevel 1 goto source_error

call :build_work
if errorlevel 1 goto runtime_error

if not exist "!target_work!\" (
    mkdir "!target_work!"
    if errorlevel 1 goto install_error
)

call :install_base
if errorlevel 1 goto install_error
call :refresh_existing_instructions
if errorlevel 1 goto install_error
call :install_selected_instructions
if errorlevel 1 goto install_error
call :install_binary
if errorlevel 1 goto install_error

echo Work skill installed in "!target_work!".
echo Existing matching files were overwritten. Stale files were not removed.
pause
exit /b 0

:find_rust_toolchain
if defined CARGO_HOME (
    if exist "!CARGO_HOME!\bin\rustc.exe" if exist "!CARGO_HOME!\bin\cargo.exe" (
        set "PATH=!CARGO_HOME!\bin;!PATH!"
        exit /b 0
    )
)
if exist "%USERPROFILE%\.cargo\bin\rustc.exe" if exist "%USERPROFILE%\.cargo\bin\cargo.exe" (
    set "PATH=%USERPROFILE%\.cargo\bin;!PATH!"
)
exit /b 0

:validate_rust_toolchain
where rustc >nul 2>nul
if errorlevel 1 call :find_rust_toolchain
where cargo >nul 2>nul
if errorlevel 1 call :find_rust_toolchain
for %%T in (rustc cargo) do (
    where %%T >nul 2>nul
    if errorlevel 1 (
        set "runtime_error_message=%%T is required. Install Rust 1.85 or newer with Cargo and try again."
        exit /b 1
    )
    for /f "tokens=2" %%V in ('%%T --version') do set "tool_version=%%V"
    for /f "tokens=1,2 delims=." %%A in ("!tool_version!") do (
        set "tool_major=%%A"
        set "tool_minor=%%B"
    )
    if !tool_major! LSS 1 (
        set "runtime_error_message=%%T 1.85 or newer is required."
        exit /b 1
    )
    if !tool_major! EQU 1 if !tool_minor! LSS 85 (
        set "runtime_error_message=%%T 1.85 or newer is required."
        exit /b 1
    )
)
for /f "tokens=2" %%T in ('rustc -vV ^| findstr /b "host:"') do set "rust_host=%%T"
if not defined rust_host (
    set "runtime_error_message=Cannot read the Rust host target."
    exit /b 1
)
echo !rust_host! | findstr /c:"-windows-msvc" >nul
if not errorlevel 1 (
    where link.exe >nul 2>nul
    if errorlevel 1 echo Warning: MSVC linker is not on PATH; Cargo will check Visual Studio Build Tools during build.
) else (
    echo !rust_host! | findstr /c:"-windows-gnu" >nul
    if errorlevel 1 (
        set "runtime_error_message=Unsupported Windows Rust host target: !rust_host!."
        exit /b 1
    )
    where gcc.exe >nul 2>nul
    if errorlevel 1 echo Warning: GNU linker is not on PATH; Cargo will check the configured linker during build.
)
exit /b 0

:build_work
pushd "%project_directory%\rust"
if errorlevel 1 (
    set "runtime_error_message=Cannot enter the Rust source directory."
    exit /b 1
)
cargo build --release --locked -p work-cli
set "build_result=!errorlevel!"
popd
if not "!build_result!"=="0" (
    set "runtime_error_message=Rust build failed. Check linker/SDK setup and crate downloads; installed Work binary was not changed."
    exit /b 1
)
set "built_work=%project_directory%\rust\target\release\work.exe"
if not exist "!built_work!" (
    set "runtime_error_message=Rust build did not produce work.exe; installed Work binary was not changed."
    exit /b 1
)
"!built_work!" --help >nul 2>nul
if errorlevel 1 (
    set "runtime_error_message=Built Work binary failed its startup check; installed Work binary was not changed."
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
call :require_file "references\instruction-loading\invocation.md"
if errorlevel 1 exit /b 1
for %%M in (plan task execute specification progress) do (
    call :require_file "references\workflows\%%M.md"
    if errorlevel 1 exit /b 1
)
for %%M in (plan task-coordinator task-skill execute artifact-editor progress-saver) do (
    call :require_file "references\subagents\%%M.md"
    if errorlevel 1 exit /b 1
)
if not exist "%project_directory%\rust\Cargo.toml" (
    set "missing_source=%project_directory%\rust\Cargo.toml"
    exit /b 1
)
if not exist "%project_directory%\rust\Cargo.lock" (
    set "missing_source=%project_directory%\rust\Cargo.lock"
    exit /b 1
)
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
    call :require_instruction "%%M" "programming-language"
    if errorlevel 1 exit /b 1
)
if defined include_typescript for %%M in (plan task execute) do (
    call :require_instruction "%%M" "programming-language"
    if errorlevel 1 exit /b 1
)
if defined include_java for %%M in (plan task execute) do (
    call :require_instruction "%%M" "programming-language\java"
    if errorlevel 1 exit /b 1
)
if defined include_spring for %%M in (task execute) do (
    call :require_instruction "%%M" "programming-language\java\spring-boot"
    if errorlevel 1 exit /b 1
)
if defined include_persistence for %%M in (task execute) do (
    call :require_instruction "%%M" "programming-language\java\persistence"
    if errorlevel 1 exit /b 1
)
if defined include_jpa for %%M in (task execute) do (
    call :require_instruction "%%M" "programming-language\java\persistence\jpa"
    if errorlevel 1 exit /b 1
)
if defined include_mybatis for %%M in (task execute) do (
    call :require_instruction "%%M" "programming-language\java\persistence\mybatis"
    if errorlevel 1 exit /b 1
)
if defined include_frontend for %%M in (plan task execute) do (
    call :require_instruction "%%M" "web\frontend"
    if errorlevel 1 exit /b 1
)
if defined include_typescript for %%M in (plan task execute) do (
    call :require_instruction "%%M" "programming-language\typescript"
    if errorlevel 1 exit /b 1
)
if defined include_astro for %%M in (task execute) do (
    call :require_instruction "%%M" "web\frontend\astro"
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
    set "include_java=1"
)
if "%~1"=="5" (
    set "include_java=1"
    set "include_persistence=1"
    set "include_jpa=1"
)
if "%~1"=="6" (
    set "include_java=1"
    set "include_persistence=1"
    set "include_mybatis=1"
)
if "%~1"=="12" (
    set "include_java=1"
    set "include_spring=1"
)
if "%~1"=="7" (
    set "include_web=1"
    set "include_frontend=1"
)
if "%~1"=="8" (
    set "include_typescript=1"
)
if "%~1"=="9" (
    set "include_web=1"
    set "include_frontend=1"
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
call :copy_tree "references\instruction-loading"
if errorlevel 1 exit /b 1
call :copy_tree "references\workflows"
if errorlevel 1 exit /b 1
call :copy_tree "references\subagents"
if errorlevel 1 exit /b 1
exit /b 0

:install_binary
if not exist "!target_work!\scripts\" mkdir "!target_work!\scripts"
if errorlevel 1 exit /b 1
set "staged_work=!target_work!\scripts\work.new.exe"
copy /Y "!built_work!" "!staged_work!" >nul
if errorlevel 1 exit /b 1
move /Y "!staged_work!" "!target_work!\scripts\work.exe" >nul
if errorlevel 1 exit /b 1
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
    call :copy_instruction "%%M" "programming-language"
    if errorlevel 1 exit /b 1
)
if defined include_typescript for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "programming-language"
    if errorlevel 1 exit /b 1
)
if defined include_java for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "programming-language\java"
    if errorlevel 1 exit /b 1
)
if defined include_spring for %%M in (task execute) do (
    call :copy_instruction "%%M" "programming-language\java\spring-boot"
    if errorlevel 1 exit /b 1
)
if defined include_persistence for %%M in (task execute) do (
    call :copy_instruction "%%M" "programming-language\java\persistence"
    if errorlevel 1 exit /b 1
)
if defined include_jpa for %%M in (task execute) do (
    call :copy_instruction "%%M" "programming-language\java\persistence\jpa"
    if errorlevel 1 exit /b 1
)
if defined include_mybatis for %%M in (task execute) do (
    call :copy_instruction "%%M" "programming-language\java\persistence\mybatis"
    if errorlevel 1 exit /b 1
)
if defined include_frontend for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "web\frontend"
    if errorlevel 1 exit /b 1
)
if defined include_typescript for %%M in (plan task execute) do (
    call :copy_instruction "%%M" "programming-language\typescript"
    if errorlevel 1 exit /b 1
)
if defined include_astro for %%M in (task execute) do (
    call :copy_instruction "%%M" "web\frontend\astro"
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

:refresh_existing_instructions
for /r "%source_work%\references\instructions" %%F in (instructions.md) do (
    set "instruction_relative=%%~fF"
    set "instruction_relative=!instruction_relative:%source_work%\=!"
    if exist "!target_work!\!instruction_relative!" (
        call :copy_file "!instruction_relative!"
        if errorlevel 1 exit /b 1
        set "branch_relative=!instruction_relative:\instructions.md=!"
        if exist "!source_work!\!branch_relative!\references\" (
            call :copy_tree "!branch_relative!\references"
            if errorlevel 1 exit /b 1
        )
    )
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
