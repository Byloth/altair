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
python3 -I tools/forge.py digest <archivio.forge>   # per voce: id, sezioni, byte, CRC-32 (confronto con altair-forge)
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

## `altair-shaderc` (crate `crates/shader`)

Traduce gli shader D3D9 del gioco in WGSL e li valida con naga.

```sh
ALTAIR_DUMP_SHADERS=private/shaders ALTAIR_FRAMES=1500 tools/run.sh   # estrae gli shader
target/release/altair-shaderc private/shaders                         # riepilogo della copertura
target/release/altair-shaderc private/shaders/vs_….bin                # il WGSL di uno shader
```

Variabili del renderer in `tools/run.sh`: `ALTAIR_SHOT_DIR`, `ALTAIR_SHOT_EVERY`,
`ALTAIR_FRAMES`, `ALTAIR_GFX=0`. Vedi `docs/analisi/d3d9.md`.

## `avvia.sh`, `ctl.sh`

Pilotano il gioco tradotto mentre gira, attraverso il canale di comando del runtime
(`ALTAIR_CTL`, cartella `run/ctl/`, ignorata da git).

```sh
tools/avvia.sh              # avvia in background; registro in run/ctl/log.txt
tools/avvia.sh --partita    # in più: titolo → profilo "Claude" → menu → CONTINUA
tools/avvia.sh --animus     # in più: laboratorio → Animus → corridoio con Altaïr (5 minuti)
tools/avvia.sh --tutorial   # in più: i primi tre passi del tutorial
tools/ctl.sh "key 28" "shot a.png"   # tasto Invio e schermata in run/ctl/a.png
tools/ctl.sh quit           # censimento ed uscita
```

Comandi (le durate sono in fotogrammi, o in tempo reale con `ms`: `key 17 3000ms`): `key DIK [durata]`, `text TESTO`, `mouse DX DY`, `button N [fotogrammi]`,
`move X Y`, `click X Y`, `shot FILE.png`, `stats`, `profile [reset]`, `fps` (accende o spegne il
pannello di diagnostica, come F8; con il pannello acceso `shot` lo comprende), `press` (vale come
un tasto qualunque: fa scorrere via un filmato iniziale), `quit`. `ctl.sh` aspetta
che il gioco li abbia eseguiti e stampa fotogramma, ritmo e filmato in corso. Non lanciare `avvia.sh` sotto
`timeout`: ucciderebbe anche il gioco. `avvia.sh` e `gioca.sh` chiudono all'avvio ogni
istanza del gioco già in esecuzione: ne gira una sola alla volta.

## `gioca.sh`

Avvia il gioco in una finestra, con tastiera, mouse e audio veri (`ALTAIR_WINDOW=1`).

```sh
tools/gioca.sh                      # chiudere la finestra per uscire
ALTAIR_AUDIO=0 tools/gioca.sh       # senza uscita audio
ALTAIR_LANG=English tools/gioca.sh  # lingua del gioco (predefinita: Italian)
```

## `audio-probe.py`

Misura se l'uscita audio consuma il suono in tempo reale, fuori dal gioco: 4 secondi di tono
a 440 Hz scritti a blocchi, con `libpulse-simple` (come fa il runtime) o con SDL2.

```sh
python3 -I tools/audio-probe.py tutto          # tutte le combinazioni utili
python3 -I tools/audio-probe.py pulse 10 200   # blocchi da 10 ms, coda da 200 ms
python3 -I tools/audio-probe.py sdl
```

Se 4 secondi di suono richiedono molto più di 4 secondi, il problema è nel server audio.

## `binktest.py`

Confronta il decodificatore Bink del progetto (`altair-bink`, crate `crates/bink`) con
`ffmpeg`, che fa da oracolo: ogni fotogramma video deve essere identico byte per byte; per
l'audio si misura l'errore. Il confronto avviene in memoria, non scrive nulla.

```sh
cargo build --release -p altair-bink
python3 -I tools/binktest.py "$AC1/videos"                       # tutti i filmati, video e audio
python3 -I tools/binktest.py "$AC1/videos/UBI_LOGO.bik" --solo-video --fotogrammi 50
```

## `build-win.sh`, `build-image-win.sh`, `win-link.sh`, `gioca.ps1`

Windows nativo. I primi tre girano in WSL e compilano per Windows (vedi `docs/ambiente.md`):

```sh
tools/build-win.sh     # build/win/altair_ac1.dll + target/x86_64-pc-windows-gnullvm/release/altair-run.exe
```

`build-image-win.sh` compila il C generato come DLL, `win-link.sh` è il linker che cargo usa
per l'eseguibile. `gioca.ps1` si lancia da PowerShell su Windows:

```powershell
powershell -ExecutionPolicy Bypass -File tools\gioca.ps1 [-Backend vulkan|dx12] [-SenzaAudio] [-SenzaVideo]
```

Chiude le istanze già aperte, avvia il gioco in finestra e lascia il registro in
`run\win\log.txt`.
