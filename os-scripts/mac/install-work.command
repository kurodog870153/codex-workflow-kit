#!/usr/bin/env bash

set -u

if (( $# != 0 )); then
    printf 'Error: this installer no longer accepts plan, task, execute, or all arguments.\n' >&2
    exit 2
fi

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
project_directory="$(cd -- "$script_directory/../.." && pwd -P)"
source_work="$project_directory/skills/work"
missing_source=""

include_web=0
include_backend=0
include_java=0
include_jpa=0
include_mybatis=0
include_frontend=0
include_typescript=0
include_astro=0
include_css=0
include_tailwind=0

validate_rust_toolchain() {
    local tool version major minor
    for tool in rustc cargo; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            printf 'Error: %s is required. Install Rust 1.85 or newer with Cargo and try again.\n' "$tool" >&2
            return 1
        fi
        version="$($tool --version)" || return 1
        if [[ ! "$version" =~ ^(rustc|cargo)[[:space:]]+([0-9]+)\.([0-9]+)\. ]]; then
            printf 'Error: cannot read %s version: %s\n' "$tool" "$version" >&2
            return 1
        fi
        major="${BASH_REMATCH[2]}"
        minor="${BASH_REMATCH[3]}"
        if (( major < 1 || (major == 1 && minor < 85) )); then
            printf 'Error: %s 1.85 or newer is required; found %s.\n' "$tool" "$version" >&2
            return 1
        fi
    done
    if ! command -v xcrun >/dev/null 2>&1 || ! xcrun --find clang >/dev/null 2>&1; then
        printf 'Error: Xcode Command Line Tools with clang are required. Install them and try again.\n' >&2
        return 1
    fi
    local sdk
    sdk="$(xcrun --show-sdk-path)" || return 1
    if [[ ! -d "$sdk" ]]; then
        printf 'Error: the macOS SDK is unavailable: %s\n' "$sdk" >&2
        return 1
    fi
}

build_work() {
    if ! (cd -- "$project_directory/rust" && cargo build --release --locked -p work-cli); then
        printf 'Error: Rust build failed. Check linker/SDK setup and crate downloads; the installed Work binary was not changed.\n' >&2
        return 1
    fi
    built_work="$project_directory/rust/target/release/work"
    if [[ ! -x "$built_work" ]] || ! "$built_work" --help >/dev/null 2>&1; then
        printf 'Error: the built Work binary failed its startup check; the installed binary was not changed.\n' >&2
        return 1
    fi
}

require_file() {
    local relative="$1"
    if [[ -f "$source_work/$relative" ]]; then
        return 0
    fi
    missing_source="$source_work/$relative"
    return 1
}

require_instruction() {
    require_file "references/instructions/$1/$2/instructions.md"
}

validate_base_sources() {
    require_file "SKILL.md" || return 1
    require_file "agents/openai.yaml" || return 1
    require_file "references/instruction-loading.md" || return 1
    require_file "references/instruction-loading/invocation.md" || return 1

    local mode
    for mode in plan task execute specification task-drafts repair progress; do
        require_file "references/workflows/$mode.md" || return 1
    done
    for mode in plan task-coordinator task-skill execute artifact-editor progress-saver; do
        require_file "references/subagents/$mode.md" || return 1
    done

    if [[ ! -f "$project_directory/rust/Cargo.toml" ]]; then
        missing_source="$project_directory/rust/Cargo.toml"
        return 1
    fi
    if [[ ! -f "$project_directory/rust/Cargo.lock" ]]; then
        missing_source="$project_directory/rust/Cargo.lock"
        return 1
    fi
}

include_hierarchy() {
    case "$1" in
        1)
            ;;
        2)
            include_web=1
            ;;
        3)
            include_web=1
            include_backend=1
            ;;
        4)
            include_web=1
            include_backend=1
            include_java=1
            ;;
        5)
            include_web=1
            include_backend=1
            include_java=1
            include_jpa=1
            ;;
        6)
            include_web=1
            include_backend=1
            include_java=1
            include_mybatis=1
            ;;
        7)
            include_web=1
            include_frontend=1
            ;;
        8)
            include_web=1
            include_frontend=1
            include_typescript=1
            ;;
        9)
            include_web=1
            include_frontend=1
            include_typescript=1
            include_astro=1
            ;;
        10)
            include_web=1
            include_frontend=1
            include_css=1
            ;;
        11)
            include_web=1
            include_frontend=1
            include_css=1
            include_tailwind=1
            ;;
    esac
}

validate_selected_instructions() {
    local mode
    for mode in plan task execute; do
        require_instruction "$mode" general || return 1
    done

    if (( include_web )); then
        for mode in plan task execute; do
            require_instruction "$mode" web || return 1
        done
    fi
    if (( include_backend )); then
        for mode in plan task execute; do
            require_instruction "$mode" web/backend || return 1
        done
    fi
    if (( include_java )); then
        for mode in plan task execute; do
            require_instruction "$mode" web/backend/java || return 1
        done
    fi
    if (( include_jpa )); then
        for mode in task execute; do
            require_instruction "$mode" web/backend/java/jpa || return 1
        done
    fi
    if (( include_mybatis )); then
        for mode in task execute; do
            require_instruction "$mode" web/backend/java/mybatis || return 1
        done
    fi
    if (( include_frontend )); then
        for mode in plan task execute; do
            require_instruction "$mode" web/frontend || return 1
        done
    fi
    if (( include_typescript )); then
        for mode in plan task execute; do
            require_instruction "$mode" web/frontend/typescript || return 1
        done
    fi
    if (( include_astro )); then
        for mode in task execute; do
            require_instruction "$mode" web/frontend/typescript/astro || return 1
        done
    fi
    if (( include_css )); then
        for mode in plan task execute; do
            require_instruction "$mode" web/frontend/css || return 1
        done
    fi
    if (( include_tailwind )); then
        for mode in task execute; do
            require_instruction "$mode" web/frontend/css/tailwind || return 1
        done
    fi
}

copy_file() {
    local relative="$1"
    local source="$source_work/$relative"
    local target="$target_work/$relative"

    mkdir -p -- "$(dirname -- "$target")" || return 1
    cp -f -- "$source" "$target"
}

copy_tree() {
    local relative="$1"
    local source="$source_work/$relative"
    local target="$target_work/$relative"

    mkdir -p -- "$target" || return 1
    cp -R -f -- "$source/." "$target/"
}

copy_instruction() {
    local mode="$1"
    local hierarchy="$2"
    local relative="references/instructions/$mode/$hierarchy"

    copy_file "$relative/instructions.md" || return 1
    if [[ -d "$source_work/$relative/references" ]]; then
        copy_tree "$relative/references" || return 1
    fi
}

install_base() {
    copy_file "SKILL.md" || return 1
    copy_file "agents/openai.yaml" || return 1
    copy_file "references/instruction-loading.md" || return 1
    copy_tree "references/instruction-loading" || return 1
    copy_tree "references/workflows" || return 1
    copy_tree "references/subagents" || return 1
}

install_binary() {
    local target="$target_work/scripts/work"
    local staged
    mkdir -p -- "$(dirname -- "$target")" || return 1
    staged="$(mktemp "$target.XXXXXX")" || return 1
    if ! cp -- "$built_work" "$staged" || ! chmod 755 "$staged" || ! mv -f -- "$staged" "$target"; then
        printf 'Error: failed to publish Work binary; previous binary remains in place.\n' >&2
        return 1
    fi
}

install_selected_instructions() {
    local mode
    for mode in plan task execute; do
        copy_instruction "$mode" general || return 1
    done

    if (( include_web )); then
        for mode in plan task execute; do
            copy_instruction "$mode" web || return 1
        done
    fi
    if (( include_backend )); then
        for mode in plan task execute; do
            copy_instruction "$mode" web/backend || return 1
        done
    fi
    if (( include_java )); then
        for mode in plan task execute; do
            copy_instruction "$mode" web/backend/java || return 1
        done
    fi
    if (( include_jpa )); then
        for mode in task execute; do
            copy_instruction "$mode" web/backend/java/jpa || return 1
        done
    fi
    if (( include_mybatis )); then
        for mode in task execute; do
            copy_instruction "$mode" web/backend/java/mybatis || return 1
        done
    fi
    if (( include_frontend )); then
        for mode in plan task execute; do
            copy_instruction "$mode" web/frontend || return 1
        done
    fi
    if (( include_typescript )); then
        for mode in plan task execute; do
            copy_instruction "$mode" web/frontend/typescript || return 1
        done
    fi
    if (( include_astro )); then
        for mode in task execute; do
            copy_instruction "$mode" web/frontend/typescript/astro || return 1
        done
    fi
    if (( include_css )); then
        for mode in plan task execute; do
            copy_instruction "$mode" web/frontend/css || return 1
        done
    fi
    if (( include_tailwind )); then
        for mode in task execute; do
            copy_instruction "$mode" web/frontend/css/tailwind || return 1
        done
    fi
}

refresh_existing_instructions() {
    local source_file relative branch
    while IFS= read -r -d '' source_file; do
        relative="${source_file#"$source_work/"}"
        if [[ ! -f "$target_work/$relative" ]]; then
            continue
        fi
        copy_file "$relative" || return 1
        branch="${relative%/instructions.md}"
        if [[ -d "$source_work/$branch/references" ]]; then
            copy_tree "$branch/references" || return 1
        fi
    done < <(find "$source_work/references/instructions" -name instructions.md -type f -print0)
}

if ! validate_rust_toolchain; then
    exit 1
fi

if ! validate_base_sources; then
    printf 'Error: required Work skill source not found: "%s".\n' "$missing_source" >&2
    exit 1
fi

while true; do
    printf 'Installation location:\n'
    printf '  1. Default installation directory: "%s/.agents"\n' "$HOME"
    printf '  2. Custom installation directory\n'
    read -r -p 'Select an installation location [1]: ' home_choice
    home_choice="${home_choice:-1}"

    if [[ "$home_choice" == "1" ]]; then
        install_home="$HOME"
        break
    fi
    if [[ "$home_choice" != "2" ]]; then
        printf 'Invalid selection. Please try again.\n'
        continue
    fi

    while true; do
        read -r -p 'Enter the installation directory: ' install_home
        if [[ "$install_home" == "~" ]]; then
            install_home="$HOME"
        elif [[ "$install_home" == "~/"* ]]; then
            install_home="$HOME/${install_home#~/}"
        fi
        if [[ -d "$install_home" ]]; then
            break
        fi
        printf 'The installation directory does not exist: "%s". Please try again.\n' "$install_home"
    done
    break
done

if [[ ! -d "$install_home" ]]; then
    printf 'Error: the selected installation directory does not exist: "%s".\n' "$install_home" >&2
    exit 1
fi
install_home="$(cd -- "$install_home" && pwd -P)"
if [[ "$home_choice" == "1" ]]; then
    install_home="$install_home/.agents"
fi
target_work="$install_home/skills/work"

while true; do
    printf 'Instruction hierarchy:\n'
    printf '  1. general only\n'
    printf '  2. web\n'
    printf '  3. backend\n'
    printf '  4. java\n'
    printf '  5. jpa\n'
    printf '  6. mybatis\n'
    printf '  7. frontend\n'
    printf '  8. typescript\n'
    printf '  9. astro\n'
    printf '  10. css\n'
    printf '  11. tailwind\n'
    printf 'Select multiple branches with spaces. Parent branches are included automatically.\n'
    printf 'Previously installed branches and stale files will be kept, even with general only.\n'
    read -r -p 'Select hierarchy numbers, enter "all", or press Enter for general only: ' hierarchy_selection
    hierarchy_selection="${hierarchy_selection:-1}"

    if [[ "$hierarchy_selection" == "all" ]]; then
        include_hierarchy 5
        include_hierarchy 6
        include_hierarchy 9
        include_hierarchy 11
        break
    fi

    read -r -a hierarchy_tokens <<< "$hierarchy_selection"
    valid_selection=1
    for token in "${hierarchy_tokens[@]}"; do
        if [[ ! "$token" =~ ^([1-9]|1[01])$ ]]; then
            valid_selection=0
            break
        fi
    done
    if (( ! valid_selection || ${#hierarchy_tokens[@]} == 0 )); then
        printf 'Invalid selection. Please try again.\n'
        continue
    fi
    for token in "${hierarchy_tokens[@]}"; do
        include_hierarchy "$token"
    done
    break
done

if ! validate_selected_instructions; then
    printf 'Error: required Work skill source not found: "%s".\n' "$missing_source" >&2
    exit 1
fi

if ! build_work; then
    exit 1
fi

if ! mkdir -p -- "$target_work"; then
    printf 'Error: failed to create the Work skill directory: "%s".\n' "$target_work" >&2
    exit 1
fi
if ! install_base || ! refresh_existing_instructions || ! install_selected_instructions || ! install_binary; then
    printf 'Error: failed to install the Work skill in "%s".\n' "$target_work" >&2
    exit 1
fi

printf 'Work skill installed in "%s".\n' "$target_work"
printf 'Existing matching files were overwritten. Stale files were not removed.\n'
