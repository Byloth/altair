#!/usr/bin/env bash
# Rinomina la cartella del progetto e porta con sé sessioni e memoria di Claude Code.
#
# Uso, a sessione di Claude Code CHIUSA:
#     bash ~/projects/ac-rek/tools/migra-cartella.sh ~/projects/ac-rek ~/projects/altair
#
# Cosa fa:
#   1. sposta la cartella del progetto;
#   2. copia le conversazioni di Claude Code (i file .jsonl) nella cartella che Claude Code
#      associa al nuovo percorso, correggendo il percorso di lavoro registrato al loro interno;
#      gli originali restano dove sono, come copia di sicurezza;
#   3. copia la memoria, senza sovrascrivere i file già presenti;
#   4. ricrea il venv Python, che contiene percorsi assoluti, e ricompila.
set -euo pipefail

main() {
    if [[ $# -ne 2 ]]; then
        sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'
        exit 1
    fi
    if [[ -n "${CLAUDECODE:-}" ]]; then
        echo "Sembra che tu sia dentro una sessione di Claude Code: chiudila e rilancia da un terminale normale." >&2
        exit 1
    fi
    local old new
    old=$(realpath "$1")
    new=$(realpath -m "$2")
    [[ -d "$old" ]] || { echo "La cartella $old non esiste." >&2; exit 1; }
    [[ ! -e "$new" ]] || { echo "$new esiste già: non sovrascrivo." >&2; exit 1; }

    # Claude Code tiene i dati di un progetto in ~/.claude/projects/<percorso con - al posto di />.
    local store="$HOME/.claude/projects"
    local old_key="$store/${old//\//-}" new_key="$store/${new//\//-}"

    echo "1. Sposto $old -> $new"
    mv "$old" "$new"

    echo "2. Conversazioni"
    mkdir -p "$new_key"
    shopt -s nullglob
    for f in "$old_key"/*.jsonl; do
        local dest="$new_key/$(basename "$f")"
        if [[ -e "$dest" ]]; then
            echo "   $(basename "$f"): già presente, salto"
            continue
        fi
        # Corregge solo il campo "cwd"; il resto della conversazione resta com'è.
        sed "s|\"cwd\":\"$old|\"cwd\":\"$new|g" "$f" > "$dest"
        chmod 600 "$dest"
        echo "   $(basename "$f") copiata"
    done
    for d in "$old_key"/*/; do
        local name
        name=$(basename "$d")
        [[ "$name" == memory ]] && continue
        [[ -e "$new_key/$name" ]] || cp -r "$d" "$new_key/$name"
    done

    echo "3. Memoria"
    mkdir -p "$new_key/memory"
    if [[ -d "$old_key/memory" ]]; then
        cp -n "$old_key/memory"/*.md "$new_key/memory/" 2>/dev/null || true
    fi

    echo "4. Ambiente Python e compilazione"
    cd "$new"
    rm -rf .venv
    python3 -m venv .venv
    .venv/bin/pip install -q unicorn capstone
    # shellcheck disable=SC1090
    [[ -f "$HOME/.bash_exports" ]] && source "$HOME/.bash_exports"
    if command -v cargo >/dev/null; then
        cargo build --release
    else
        echo "   cargo non trovato: esegui 'cargo build --release' a mano."
    fi

    echo
    echo "Fatto. Per riprendere la conversazione:"
    echo "    cd $new && claude --resume"
    echo "Gli originali restano in $old_key: cancellali quando hai verificato che tutto funziona."
}

main "$@"
