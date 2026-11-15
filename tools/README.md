# Strumenti

Tutti gli script leggono i file del gioco e non scrivono mai nella sua cartella.

```sh
export AC1="/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed"
```

## `pe.py`

Analisi statica di PE32, solo libreria standard.

```sh
python3 -I tools/pe.py info    "$AC1/AssassinsCreed_Dx9.exe"   # hash, header, sezioni, import, RTTI, build string
python3 -I tools/pe.py imports "$AC1/AssassinsCreed_Dx9.exe"   # elenco funzioni importate per DLL
python3 -I tools/pe.py diff    "$AC1/AssassinsCreed_Dx9.exe" "$AC1/AssassinsCreed_Dx10.exe"
python3 -I tools/pe.py rtti    "$AC1/AssassinsCreed_Dx9.exe"   # nomi di tipo RTTI decorati
```

`rtti` è una scansione veloce di stringhe (4.376 nomi nella DX9). Per gerarchie e vtable
usa `rtti.py`. L'output contiene nomi di classi Ubisoft: se lo salvi, mettilo in `private/`.

## `rtti.py`

Parser dell'RTTI di MSVC a 32 bit: type descriptor, complete object locator, gerarchie
delle classi, vtable con i loro slot.

```sh
python3 -I tools/rtti.py "$AC1/AssassinsCreed_Dx9.exe"                               # statistiche
python3 -I tools/rtti.py "$AC1/AssassinsCreed_Dx9.exe" --out private/rtti-dx9.json   # dettaglio
```

Il JSON ha tre liste: `types` (indirizzo, nome decorato e leggibile), `hierarchies` (basi di
ogni classe con gli spiazzamenti) e `vtables` (indirizzo, classe, offset, slot). Risultati in
`docs/analisi/rtti.md`.

## `imports.py`

Mappa degli import: nome (ordinali risolti), cella IAT, riferimenti in `.text`, categoria.

```sh
python3 -I tools/imports.py "$AC1/AssassinsCreed_Dx9.exe"          # tabelle Markdown
python3 -I tools/imports.py "$AC1/AssassinsCreed_Dx9.exe" --guids  # GUID COM noti presenti
```

Le tabelle di `docs/analisi/import.md` vengono da qui.

## `functions.py`

Scoperta delle funzioni senza simboli (prototipo). Serve Capstone, quindi il Python del venv.

```sh
.venv/bin/python tools/functions.py "$AC1/AssassinsCreed_Dx9.exe" --out private/functions-dx9.json
.venv/bin/python tools/functions.py "$AC1/AssassinsCreed_Dx9.exe" --strict   # regola più severa
```

Stampa le statistiche in JSON; con `--out` salva l'elenco `[indirizzo, origine]`. Circa 30
secondi. Metodo, risultati e limiti in `docs/analisi/funzioni.md`.

## `harness.py`

Esegue funzioni dell'eseguibile originale in Unicorn: l'oracolo dei test differenziali.

```sh
.venv/bin/python tools/harness.py "$AC1/AssassinsCreed_Dx9.exe" selftest
.venv/bin/python tools/harness.py "$AC1/AssassinsCreed_Dx9.exe" call 0xFA6790 s:Altair
```

Uso da Python, mappa della memoria e limiti in `docs/analisi/harness.md`.

## `forge.py`

Lettore degli archivi `.forge` (contenitore versione 25), con decompressore LZO1X in Python.

```sh
python3 -I tools/forge.py summary "$AC1"                                  # una riga per archivio
python3 -I tools/forge.py list "$AC1/DataPC.forge"                        # id, dimensione, nome
python3 -I tools/forge.py sections "$AC1/DataPC.forge" 20                 # sezioni compresse
python3 -I tools/forge.py head "$AC1/DataPC_Map_Menu.forge" Map_Menu      # hexdump dell'indice
python3 -I tools/forge.py classes "$AC1/DataPC.forge" private/rtti-dx9.json [--objects]
```

`classes` conta gli oggetti per classe, traducendo i CRC-32 con i nomi di `rtti.py`. È lento
sugli archivi grandi (quasi 3 minuti per `DataPC.forge`). Gli output contengono nomi e dati del
gioco: vanno in `private/`. Formato e risultati in `docs/analisi/forge.md`.

## `difftest.py`

Test differenziale: traduce con `altair`, compila con clang, esegue ogni scenario sull'originale
(Unicorn) e sul tradotto, confronta registri, stack, import chiamati e memoria.

```sh
source ~/.bash_exports && cargo build --release
.venv/bin/python tools/difftest.py "$AC1/AssassinsCreed_Dx9.exe"
```

Gli scenari sono funzioni Python in fondo al file. Dettagli in `docs/analisi/traduttore.md`.

## `difftrace.py`

Rigioca un caso di `difftest.py --auto` sull'originale (con la traccia degli indirizzi
eseguiti) e sul tradotto, e mostra registri e prime differenze di heap e stack.

```sh
.venv/bin/python tools/difftrace.py "$AC1/AssassinsCreed_Dx9.exe" 0x99BC40 1
```

## `migra-cartella.sh`

Rinomina o sposta la cartella del progetto portando con sé conversazioni e memoria di Claude
Code, poi ricrea il venv e ricompila. Va lanciato a sessione chiusa. Vedi `docs/ambiente.md`.

`difftest.py --funcs 0xA,0xB --cases 40` prova funzioni scelte a mano, anche non candidate.
`difftest.py --auto N --lib build/libaltair_ac1-clang.so` usa la libreria dell'immagine intera.

## `build-image.sh`, `run.sh`

`tools/build-image.sh` rigenera l'immagine tradotta (`altair image`) e ricompila solo i file
cambiati; accetta argomenti per `make` (es. `CC=gcc`). `tools/run.sh [--trace]` avvia il gioco
tradotto con `altair-run`. Vedi `docs/analisi/runtime.md`.

## `altair-gpu-probe` (crate `crates/gpu-probe`)

Prove della catena grafica: elenco degli adattatori, `triangle FILE.png` (fuori schermo, con
rilettura), `window [secondi]` (finestra vera). Per la GPU in WSL:
`GALLIUM_DRIVER=d3d12 LD_LIBRARY_PATH=/usr/lib/wsl/lib`. Vedi `docs/analisi/d3d9.md`.
