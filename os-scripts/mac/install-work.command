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

validate_python_runtime() {
    local candidate
    for candidate in python3 python; do
        if ! command -v "$candidate" >/dev/null 2>&1; then
            continue
        fi
        if ! "$candidate" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 10) else 1)' </dev/null >/dev/null 2>&1; then
            continue
        fi
        if ! "$candidate" -c 'import yaml' </dev/null >/dev/null 2>&1; then
            printf 'Error: PyYAML is required. Install PyYAML for %s and run this installer again.\n' "$candidate" >&2
            printf 'The installer does not install Python packages automatically.\n' >&2
            return 1
        fi
        return 0
    done

    printf 'Error: Python 3.10 or newer is required. Install Python and run this installer again.\n' >&2
    printf 'The installer does not install Python packages automatically.\n' >&2
    return 1
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

    local mode
    for mode in plan task execute; do
        require_file "references/workflows/$mode.md" || return 1
        require_file "references/subagents/$mode.md" || return 1
    done

    require_file "scripts/work.py" || return 1
    require_file "scripts/worklib/cli.py" || return 1
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
    copy_tree "references/workflows" || return 1
    copy_tree "references/subagents" || return 1
    copy_file "scripts/work.py" || return 1

    local source_file
    local relative
    while IFS= read -r -d '' source_file; do
        relative="${source_file#"$source_work/"}"
        if [[ "$source_file" == */__pycache__/* || "${source_file##*/}" == "rules.py" ]]; then
            continue
        fi
        copy_file "$relative" || return 1
    done < <(find "$source_work/scripts/worklib" -type f -name '*.py' -print0)
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

if ! validate_python_runtime; then
    exit 1
fi

if ! validate_base_sources; then
    printf 'Error: required Work skill source not found: "%s".\n' "$missing_source" >&2
    exit 1
fi

while true; do
    printf 'Installation location:\n'
    printf '  1. Default user directory: "%s"\n' "$HOME"
    printf '  2. Custom user directory\n'
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
        read -r -p 'Enter the user directory: ' install_home
        if [[ "$install_home" == "~" ]]; then
            install_home="$HOME"
        elif [[ "$install_home" == "~/"* ]]; then
            install_home="$HOME/${install_home#~/}"
        fi
        if [[ -d "$install_home" ]]; then
            break
        fi
        printf 'The user directory does not exist: "%s". Please try again.\n' "$install_home"
    done
    break
done

if [[ ! -d "$install_home" ]]; then
    printf 'Error: the selected user directory does not exist: "%s".\n' "$install_home" >&2
    exit 1
fi
install_home="$(cd -- "$install_home" && pwd -P)"
target_work="$install_home/.agents/skills/work"

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

if ! mkdir -p -- "$target_work"; then
    printf 'Error: failed to create the Work skill directory: "%s".\n' "$target_work" >&2
    exit 1
fi
if ! install_base || ! install_selected_instructions; then
    printf 'Error: failed to install the Work skill in "%s".\n' "$target_work" >&2
    exit 1
fi

printf 'Work skill installed in "%s".\n' "$target_work"
printf 'Existing matching files were overwritten. Stale files were not removed.\n'
