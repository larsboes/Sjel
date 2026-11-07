#!/usr/bin/env bash

sjel_replace_directory() {
  local incoming="$1" destination="$2" backup="${2}.previous.$$"
  if [ -e "$backup" ]; then
    echo "replacement: backup path already exists: $backup" >&2
    return 1
  fi
  if [ -e "$destination" ]; then
    mv "$destination" "$backup" || return 1
  fi
  if ! mv "$incoming" "$destination"; then
    # A cross-filesystem move can leave a partial destination behind. Remove only that failed
    # incoming tree before restoring the previous directory, or mv may nest the backup inside it.
    if [ -e "$destination" ]; then
      rm -rf "$destination" || {
        echo "replacement: remove the partial destination at $destination before retrying" >&2
        return 1
      }
    fi
    if [ -e "$backup" ]; then
      mv "$backup" "$destination" || {
        echo "replacement: restore the previous directory from $backup" >&2
        return 1
      }
    fi
    return 1
  fi
  if [ -e "$backup" ]; then
    rm -rf "$backup" || echo "replacement: previous directory retained at $backup" >&2
  fi
}
