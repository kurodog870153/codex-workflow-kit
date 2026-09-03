#!/bin/bash

script_directory="$(cd -- "$(dirname -- "$0")" && pwd)"
project_directory="$(cd -- "$script_directory/../.." && pwd)"
source_file="$project_directory/AGENTS.md"

if [[ ! -f "$source_file" ]]; then
    printf 'Error: source file not found: "%s".\n' "$source_file"
    read -r -p 'Press Enter to close...' _
    exit 1
fi

while true; do
    read -r -p 'Enter the target directory: ' target_directory

    if [[ -z "$target_directory" ]]; then
        printf 'The directory cannot be empty. Please try again.\n'
        continue
    fi

    if [[ "$target_directory" == "~" ]]; then
        target_directory="$HOME"
    elif [[ "$target_directory" == ~/* ]]; then
        target_directory="$HOME/${target_directory#~/}"
    fi

    break
done

if [[ ! -d "$target_directory" ]]; then
    read -r -p 'The directory does not exist. Create it? [y/N] ' create_directory
    if [[ ! "$create_directory" =~ ^[Yy]$ ]]; then
        printf 'Operation cancelled.\n'
        read -r -p 'Press Enter to close...' _
        exit 0
    fi

    if ! mkdir -p -- "$target_directory"; then
        printf 'Error: failed to create the directory "%s".\n' "$target_directory"
        read -r -p 'Press Enter to close...' _
        exit 1
    fi
fi

target_file="$target_directory/AGENTS.md"

if [[ -e "$target_file" ]]; then
    read -r -p 'AGENTS.md already exists in the target directory. Overwrite it? [y/N] ' overwrite_file
    if [[ ! "$overwrite_file" =~ ^[Yy]$ ]]; then
        printf 'Operation cancelled.\n'
        read -r -p 'Press Enter to close...' _
        exit 0
    fi
fi

if ! cp -f -- "$source_file" "$target_file"; then
    printf 'Error: failed to copy AGENTS.md.\n'
    read -r -p 'Press Enter to close...' _
    exit 1
fi

printf 'AGENTS.md was copied to "%s".\n' "$target_directory"
read -r -p 'Press Enter to close...' _
