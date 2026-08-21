#!/bin/sh
set -eu

usage() {
    printf '%s\n' "usage: $0 DATABASE_PATH BACKUP_PATH" >&2
    exit 2
}

[ "$#" -eq 2 ] || usage
database=$1
backup=$2
[ -f "$database" ] || { printf '%s\n' "database does not exist" >&2; exit 1; }
[ ! -e "$backup" ] || { printf '%s\n' "backup path already exists" >&2; exit 1; }
mkdir -p "$(dirname "$backup")"
temporary="${backup}.tmp.$$"
trap 'rm -f "$temporary"' EXIT HUP INT TERM
cp -p "$database" "$temporary"
sync "$temporary"
mv "$temporary" "$backup"
trap - EXIT HUP INT TERM
printf '%s\n' "$backup"
