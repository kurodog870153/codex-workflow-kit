#!/bin/bash

codex_directory="${HOME}/.codex"

echo "Codex directory: ${codex_directory}"
echo "This will permanently delete:"
echo "  sessions"
echo "  archived_sessions"
echo "  generated_images"
echo "  session_index.jsonl"
echo "  history.jsonl"
echo "  state_5.sqlite"
echo "  state_5.sqlite-shm"
echo "  state_5.sqlite-wal"
echo "Close Codex before continuing."

if [ ! -d "${codex_directory}" ]; then
    echo "No Codex directory was found. Nothing to clean."
    read -r -p "Press Enter to close..."
    exit 0
fi

printf "Permanently delete all local Codex data listed above? [y/N] "
read -r confirmation

case "${confirmation}" in
    y|Y)
        ;;
    *)
        echo "Operation cancelled."
        read -r -p "Press Enter to close..."
        exit 0
        ;;
esac

cleanup_failed=0

rm -rf "${codex_directory}/sessions" || cleanup_failed=1
rm -rf "${codex_directory}/archived_sessions" || cleanup_failed=1
rm -rf "${codex_directory}/generated_images" || cleanup_failed=1
rm -f "${codex_directory}/session_index.jsonl" || cleanup_failed=1
rm -f "${codex_directory}/history.jsonl" || cleanup_failed=1
rm -f "${codex_directory}/state_5.sqlite" || cleanup_failed=1
rm -f "${codex_directory}/state_5.sqlite-shm" || cleanup_failed=1
rm -f "${codex_directory}/state_5.sqlite-wal" || cleanup_failed=1

if [ "${cleanup_failed}" -ne 0 ]; then
    echo "Error: one or more local Codex data files could not be removed."
    read -r -p "Press Enter to close..."
    exit 1
fi

echo "Local Codex data was cleaned successfully."
read -r -p "Press Enter to close..."
