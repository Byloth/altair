#!/usr/bin/env bash
# Renames the project folder and brings Claude Code's sessions and memory along with it.
#
# Usage, with the Claude Code session CLOSED:
#     bash ~/projects/ac-rek/tools/migrate-folder.sh ~/projects/ac-rek ~/projects/altair
#
# What it does:
#   1. moves the project folder;
#   2. copies the Claude Code conversations (the .jsonl files) into the folder Claude Code
#      associates with the new path, fixing the working path recorded inside them;
#      the originals stay where they are, as a backup;
#   3. copies the memory, without overwriting files already present;
#   4. recreates the Python venv, which contains absolute paths, and rebuilds.
set -euo pipefail

main() {
    if [[ $# -ne 2 ]]; then
        sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
        exit 1
    fi
    if [[ -n "${CLAUDECODE:-}" ]]; then
        echo "You seem to be inside a Claude Code session: close it and run again from a normal terminal." >&2
        exit 1
    fi
    local old new
    old=$(realpath "$1")
    new=$(realpath -m "$2")
    [[ -d "$old" ]] || { echo "The folder $old does not exist." >&2; exit 1; }
    [[ ! -e "$new" ]] || { echo "$new already exists: not overwriting." >&2; exit 1; }

    # Claude Code keeps a project's data in ~/.claude/projects/<path with - in place of />.
    local store="$HOME/.claude/projects"
    local old_key="$store/${old//\//-}" new_key="$store/${new//\//-}"

    echo "1. Moving $old -> $new"
    mv "$old" "$new"

    echo "2. Conversations"
    mkdir -p "$new_key"
    shopt -s nullglob
    for f in "$old_key"/*.jsonl; do
        local dest="$new_key/$(basename "$f")"
        if [[ -e "$dest" ]]; then
            echo "   $(basename "$f"): already present, skipping"
            continue
        fi
        # Fixes only the "cwd" field; the rest of the conversation stays as it is.
        sed "s|\"cwd\":\"$old|\"cwd\":\"$new|g" "$f" > "$dest"
        chmod 600 "$dest"
        echo "   $(basename "$f") copied"
    done
    for d in "$old_key"/*/; do
        local name
        name=$(basename "$d")
        [[ "$name" == memory ]] && continue
        [[ -e "$new_key/$name" ]] || cp -r "$d" "$new_key/$name"
    done

    echo "3. Memory"
    mkdir -p "$new_key/memory"
    if [[ -d "$old_key/memory" ]]; then
        cp -n "$old_key/memory"/*.md "$new_key/memory/" 2>/dev/null || true
    fi

    echo "4. Python environment and build"
    cd "$new"
    rm -rf .venv
    python3 -m venv .venv
    .venv/bin/pip install -q unicorn capstone
    # shellcheck disable=SC1090
    [[ -f "$HOME/.bash_exports" ]] && source "$HOME/.bash_exports"
    if command -v cargo >/dev/null; then
        cargo build --release
    else
        echo "   cargo not found: run 'cargo build --release' by hand."
    fi

    echo
    echo "Done. To resume the conversation:"
    echo "    cd $new && claude --resume"
    echo "Originals remain in $old_key: delete them once you have verified that everything works."
}

main "$@"
