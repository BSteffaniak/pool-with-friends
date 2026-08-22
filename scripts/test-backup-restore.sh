#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
tmp=$(mktemp -d "${TMPDIR:-/tmp}/pwmtf-backup-restore.XXXXXX")
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

for tool in sqlite3 cmp; do
    command -v "$tool" >/dev/null 2>&1 || {
        printf '%s\n' "backup/restore self-test requires $tool" >&2
        exit 1
    }
done

database="$tmp/source.db"
backup="$tmp/backup.db"
restore="$tmp/restore.db"

sqlite3 "$database" <<'SQL'
PRAGMA journal_mode=WAL;
CREATE TABLE canonical_records (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);
BEGIN IMMEDIATE;
INSERT INTO canonical_records(payload) VALUES ('revision-1'), ('revision-2');
COMMIT;
SQL

"$root/scripts/backup-database.sh" "$database" "$backup" >/dev/null
"$root/scripts/restore-database.sh" "$backup" "$restore" >/dev/null

[ "$(sqlite3 "$restore" 'SELECT group_concat(payload, ",") FROM canonical_records ORDER BY id')" = "revision-1,revision-2" ]
[ "$(sqlite3 "$restore" 'PRAGMA quick_check')" = "ok" ]

# Canonical SQLite backups may have different page layout, so compare logical dumps.
sqlite3 "$backup" .dump >"$tmp/backup.dump"
sqlite3 "$restore" .dump >"$tmp/restore.dump"
cmp "$tmp/backup.dump" "$tmp/restore.dump"

if "$root/scripts/backup-database.sh" "$database" "$backup" >/dev/null 2>&1; then
    printf '%s\n' "backup unexpectedly overwrote existing output" >&2
    exit 1
fi
if "$root/scripts/restore-database.sh" "$backup" "$restore" >/dev/null 2>&1; then
    printf '%s\n' "restore unexpectedly overwrote existing output" >&2
    exit 1
fi

ln -s "$database" "$tmp/source-link.db"
if "$root/scripts/backup-database.sh" "$tmp/source-link.db" "$tmp/link-backup.db" >/dev/null 2>&1; then
    printf '%s\n' "backup unexpectedly followed a source symlink" >&2
    exit 1
fi
ln -s "$backup" "$tmp/backup-link.db"
if "$root/scripts/restore-database.sh" "$tmp/backup-link.db" "$tmp/link-restore.db" >/dev/null 2>&1; then
    printf '%s\n' "restore unexpectedly followed a backup symlink" >&2
    exit 1
fi
ln -s "$tmp/missing.db" "$tmp/output-link.db"
if "$root/scripts/backup-database.sh" "$database" "$tmp/output-link.db" >/dev/null 2>&1; then
    printf '%s\n' "backup unexpectedly replaced an output symlink" >&2
    exit 1
fi
if "$root/scripts/restore-database.sh" "$backup" "$tmp/output-link.db" >/dev/null 2>&1; then
    printf '%s\n' "restore unexpectedly replaced an output symlink" >&2
    exit 1
fi
if "$root/scripts/backup-database.sh" "$database" "$tmp/quote'backup.db" >/dev/null 2>&1; then
    printf '%s\n' "backup unexpectedly accepted a quote in a database path" >&2
    exit 1
fi
if "$root/scripts/restore-database.sh" "$backup" "$tmp/quote'restore.db" >/dev/null 2>&1; then
    printf '%s\n' "restore unexpectedly accepted a quote in a database path" >&2
    exit 1
fi
unsafe_path=$(printf '%s/control\nbackup.db' "$tmp")
if "$root/scripts/backup-database.sh" "$database" "$unsafe_path" >/dev/null 2>&1; then
    printf '%s\n' "backup unexpectedly accepted a control character in a database path" >&2
    exit 1
fi
unsafe_path=$(printf '%s/control\rrestore.db' "$tmp")
if "$root/scripts/restore-database.sh" "$backup" "$unsafe_path" >/dev/null 2>&1; then
    printf '%s\n' "restore unexpectedly accepted a control character in a database path" >&2
    exit 1
fi
mkdir "$tmp/physical-output"
ln -s "$tmp/physical-output" "$tmp/output-directory-link"
if "$root/scripts/backup-database.sh" "$database" "$tmp/output-directory-link/backup.db" >/dev/null 2>&1; then
    printf '%s\n' "backup unexpectedly traversed a symlink directory" >&2
    exit 1
fi
if "$root/scripts/restore-database.sh" "$backup" "$tmp/output-directory-link/restore.db" >/dev/null 2>&1; then
    printf '%s\n' "restore unexpectedly traversed a symlink directory" >&2
    exit 1
fi

permissions() {
    if stat -f '%Lp' "$1" >/dev/null 2>&1; then
        stat -f '%Lp' "$1"
    else
        stat -c '%a' "$1"
    fi
}

[ "$(permissions "$backup")" = 600 ]
[ "$(permissions "$restore")" = 600 ]

printf '%s\n' "application-consistent backup/restore self-test passed"
