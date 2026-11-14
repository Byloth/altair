# AC1 Recompiled (`altair`)

Ricompilazione statica di Assassin's Creed (2007), build Steam DX9, verso nativo e browser.
Piano completo, per chi legge da fuori: documento "AC1 Recompiled: piano di progetto"
(https://claude.ai/artifact/SoPWUhJZPJ45oWk3t4r8bu). Qui dentro vive il lavoro operativo.

**Nomi.** "AC1 Recompiled" è il nome completo del progetto; **Altair** è il nome tecnico:
binario `altair`, crate `altair-*`, prefisso `altair_` nel codice generato, cartella `altair`.
Un eventuale ricompilatore per un altro gioco prenderebbe il nome del suo protagonista.
Fino al 2026-11-15 il nome tecnico era `acrek` e la cartella `ac-rek`: i commit più vecchi li
citano ancora.

## Regola principale: si scrive tutto qui

Ogni scoperta, misura, decisione o ipotesi va annotata in questa directory **nello stesso
momento in cui la si fa**, non a fine sessione.

- Fatti verificati sui binari → `docs/analisi/` (un file per argomento).
- Decisioni e motivazioni → `docs/decisioni.md` (nuova voce in cima, con data).
- Domande ancora aperte → `docs/domande-aperte.md`; quando trovano risposta, spostale
  nel file di analisi giusto e lascia un rimando.
- Cosa si è fatto in ogni sessione → `docs/diario.md`.
- Avanzamento → spunta le caselle in `docs/PIANO.md`.
- Link e materiale esterno → `docs/fonti.md`.

Distingui sempre ciò che è **verificato qui** (con lo strumento o il comando che lo prova) da
ciò che viene da **fonti esterne** o è un'**ipotesi**. Scrivi in italiano.

## Percorsi

**Tutto il lavoro sta dentro questa directory**, anche ciò che non va committato: in quel
caso lo si mette in una cartella ignorata da `.gitignore`.

| Cartella | Contenuto | In git |
|---|---|---|
| `ref/` | Repository di riferimento clonati (scimitar-recomp, EaglePatch). Si leggono, non si copiano | no |
| `private/` | Materiale derivato da binari Ubisoft (nomi, dump, progetto Ghidra) | no |
| `build/` | Output generato (C tradotto, oggetti) | no |
| `.venv/`, `opt/` | venv Python, strumenti scaricati (es. Ghidra) | no |

- Gioco (Windows): `E:\Giochi\Steam\steamapps\common\Assassins Creed`
- Da WSL: `/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed` → nei comandi usa `$AC1`
  (`export AC1="/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed"`).
- Eseguibile di riferimento: `AssassinsCreed_Dx9.exe`, SHA-1 `a9ea476e70f076709cca59a802e26ec894432120`.

La cartella del gioco è **solo in lettura**: non scriverci, non copiarne file qui.

## Regole legali (dal piano, non negoziabili)

1. Nessun asset e nessun eseguibile Ubisoft nel repository. Niente dump, estratti o hexdump lunghi.
2. Il codice C generato resta in `build/` (ignorato), mai pubblicato.
3. Nomi ricavati da binari Ubisoft (es. AC2 Mac) solo in `private/` (ignorato).
4. Niente codice GPL copiato (scimitar-recomp): si legge come specifica, si riscrive.
5. Niente codice di EaglePatch (senza licenza): solo conoscenza.

Nei documenti vanno bene indirizzi, hash, nomi di funzioni/classi, conteggi e brevi citazioni
necessarie a spiegare una scoperta.

## Strumenti

```sh
source ~/.bash_exports                     # cargo non è nel PATH delle shell non interattive
cargo build --release                      # produce target/release/altair
.venv/bin/python tools/harness.py "$AC1/AssassinsCreed_Dx9.exe" selftest
.venv/bin/python tools/difftest.py "$AC1/AssassinsCreed_Dx9.exe"   # da rifare dopo ogni modifica al traduttore
```

Ogni istruzione nuova nel traduttore va accompagnata da uno scenario nel test differenziale.

- Python solo con `python3 -I` e libreria standard, salvo dipendenze dichiarate in `tools/README.md`.
- Ogni strumento nuovo va in `tools/` e documentato in `tools/README.md`.
- Stack previsto: Rust (ricompilatore e runtime, `iced-x86`, `wgpu`), C generato compilato con clang.
