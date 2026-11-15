# Piano di lavoro

Versione operativa del documento di progetto
([AC1 Recompiled - Altaïr: piano di progetto](https://claude.ai/artifact/SoPWUhJZPJ45oWk3t4r8bu)).
Ogni fase si chiude con un **gate** verificabile; non si passa alla fase successiva senza.
Spunta le caselle man mano; ogni risultato va scritto in `analisi/`, `decisioni.md` o `diario.md`.

```
0 Fondamenta → 1 Ricompilatore → 2 Runtime Win32 → 3 Grafica/audio/input
  → 4 GIOCABILE IN NATIVO (gate principale) → 5 Browser → 6 Decompilazione mirata e mod
```

---

## Fase 0 · Fondamenta ✅

**Gate:** build Steam mappata. Elenco iniziale delle funzioni, RTTI e vtable contati e
nominati, mappa completa degli import, una funzione del gioco eseguita in Unicorn da script.

### 0.1 Inventario della build ✅

- [x] Installare la build Steam e calcolare gli hash → `analisi/eseguibili.md`
- [x] Header PE, sezioni, import di DX9 e DX10 (`tools/pe.py`)
- [x] Confronto DX9 / DX10 e scelta della DX9 → `decisioni.md`
- [x] Identificare launcher e file accessori (`AssassinsCreed_Game.exe`, `Detection/`)

### 0.2 Ambiente di lavoro ✅

Completata il 2026-11-14 (WSL, 28 core, 31 GB RAM). Dettagli in `ambiente.md`.

- [x] `git init` del repository e primo commit
- [x] Rust 1.100.0 (in `~/.local/share/cargo`)
- [x] clang/LLVM 18 + lld
- [x] `.venv/` con unicorn 2.1.4 e capstone 5.0.7
- [x] Java 25 + Ghidra 12.2 in `opt/` (progetto Ghidra in `private/ghidra/`, da creare)
- [x] Clonare scimitar-recomp ed EaglePatch in `ref/` (ignorata da git)

### 0.3 La specifica: scimitar-recomp ed EaglePatch ✅

- [x] Leggere `docs/AC1_WINDOWS.md` e `docs/PLAN.md` → `analisi/scimitar-recomp.md`
- [x] Leggere `README`, `PC_DATA.md`, `ACR_WINDOWS.md`, `METAL.md` (parti utili),
  `ANVIL_FAMILY.md` (confronto), `recomp/README.md`, `platform/README.md`
- [x] Estrarre da EaglePatch gli indirizzi Steam già noti → `analisi/indirizzi-noti.md`
- [x] Confrontare il layout della loro build con la nostra (entry point, sezioni, TLS, cookie)

### 0.4 Steam contro Ubisoft Connect ✅

- [x] Layout identico: stesso codice agli stessi indirizzi, Ubisoft Connect aggiunge solo `.nidata`
- [x] Esito e conseguenze → `decisioni.md`
- [ ] (facoltativo) Confronto byte per byte di `.text`, se un giorno avremo la loro build

### 0.5 Tipi: RTTI e vtable ✅

- [x] `tools/rtti.py`: Complete Object Locator, Type Descriptor, Class Hierarchy Descriptor,
  vtable con indirizzo e slot
- [x] Conteggi identici a scimitar-recomp: 4.376 descrittori, 4.597 vtable, 59.899 slot,
  28.717 funzioni
- [x] Dettaglio in `private/rtti-dx9.json`; statistiche e sottosistemi in `analisi/rtti.md`

### 0.6 Import e superficie del sistema operativo ✅

- [x] `tools/imports.py` e `analisi/import.md`: 260 funzioni con cella IAT, riferimenti nel
  codice, categoria, modulo del runtime che le implementerà e fase
- [x] Interfacce COM richieste, dai GUID in `.rdata` (DirectInput 8, DirectSound 8 + 3D, EAX)
- [ ] Metodi COM effettivamente chiamati: rimandato al runtime (stub tracciati, Fase 2–3)

### 0.7 Primo elenco di funzioni ✅ (prototipo)

- [x] `tools/functions.py`: partenze forti (entry, export, TLS, chiamate dirette) e deboli
  (vtable, immediati, parole nei dati) con decodifica di prova
- [x] 115.807 partenze e 3,19 milioni di istruzioni (96.781 con `--strict`), contro le 98.470
  funzioni di scimitar-recomp → `analisi/funzioni.md`
- [x] Controllo incrociato con gli indirizzi di EaglePatch e con gli slot delle vtable
- [ ] Importare RTTI e funzioni in Ghidra per avere una vista navigabile (spostato: si fa
  quando servirà leggere codice, non blocca il gate)

### 0.8 Harness Unicorn ✅

- [x] `tools/harness.py`: immagine a `0x400000`, stack, TEB e `fs:` (tramite GDT), import
  intercettati da trampolini con gestori in Python
- [x] `selftest`: `strlen`, `strcmp` e `memcpy` del CRT eseguiti dall'originale e confrontati
  con Python; una funzione del motore con prologo SEH ritorna correttamente
- [x] `analisi/harness.md`: uso, struttura, limiti
- [ ] Prima prova su codice del motore (CRC-32 dei nomi di classe): va trovata la funzione

### 0.10 Dati del gioco (ricognizione) ✅

- [x] `tools/forge.py`: contenitore v25, tabelle, record, sezioni LZO1X, pacchetti di oggetti
- [x] Inventario dei 18 archivi (15.906 voci, 6,45 GB) → `analisi/forge.md`
- [x] Le classi degli oggetti sono CRC-32 dei nomi RTTI: 100% riconosciute su 3 archivi
- [x] "Missioni: dati o codice?" → struttura nei dati (27 classi `MissionItem*`, circa 950
  oggetti), comportamento nel codice

**Fase 0 chiusa il 2026-11-14.** Gate soddisfatto: build mappata, funzioni, RTTI e vtable,
import, esecuzione in Unicorn, formato dei dati.

---

## Fase 1 · Ricompilatore (gate raggiunto il 2026-11-15)

**Gate:** tutto `.text` tradotto in C che compila; test differenziali verdi su un campione
ampio di funzioni. Stato e numeri in `analisi/traduttore.md`.

### 1.1 Fondamenta ✅

- [x] Workspace Rust: `crates/pe`, `crates/recomp` (binario `altair`)
- [x] Parser PE con verifica dell'hash della build Steam
- [x] RTTI e scoperta delle funzioni portate in Rust (0,35 s; limite delle jump table dal `cmp`)
- [x] Traduttore x86 → C per le istruzioni intere, stringhe, salti, chiamate, jump table
- [x] Supporto C del codice generato (`runtime/altair.h`, `altair.c`)
- [x] Primo test differenziale contro Unicorn: `strlen`, `strcmp`, `memcpy` identici
- [x] `altair coverage`: 85,9% delle funzioni (59,5% delle istruzioni) si traduce

### 1.2 Copertura delle istruzioni

- [x] Prefisso `lock`, `xadd`, `cmpxchg`, `bts`/`btr`/`btc`: atomiche vere sulla memoria host
- [x] SSE/SSE2 scalare e impacchettato, MXCSR: copertura al 94,3% delle funzioni, resta non
  traducibile il 2,9% delle istruzioni
- [x] x87 a 80 bit con stack, confronti, control word: 99,96% delle funzioni si traduce
- [x] `shld`/`shrd`, `rcl`/`rcr`, `rdtsc`, `cpuid`, `pushfd`/`popfd`, `sahf`/`lahf`,
  `mul`/`div` a 8 e 16 bit, `bsf`/`bsr` (copertura: 89,5% delle funzioni)
- [ ] Scenari mirati per le istruzioni tradotte ma non ancora verificate (`rcl`, `bt*`,
  `bsf`/`bsr`, `pushfd`/`popfd`, `sahf`/`lahf`, `cmpsb`, `lodsb`)
- [x] MMX (mixer audio) e le ultime istruzioni rare: **tutte le 116.077 funzioni si traducono**

### 1.3 Verifica

- [x] Scelta automatica delle funzioni per il test differenziale (`altair candidates`) e
  ingressi generati (`difftest.py --auto`)
- [x] Confronto di registri XMM, heap, stack e dati globali; tolleranza sui soli NaN
- [x] Confronto dello stato x87 (cima dello stack e registri a 80 bit)
- [ ] Confronto dei flag; stack x87 non vuoto in ingresso; x87 trascendenti
- [x] Campione ampio: 4.201 funzioni del gioco identiche all'originale su 8.000, 0 diverse
- [ ] Ingressi strutturati per le funzioni oggi saltate (più di metà del campione)

### 1.4 Immagine intera

- [ ] Risolvere i limiti della scoperta (`analisi/funzioni.md`): funzioni che non ritornano,
  tabelle delle eccezioni C++
- [ ] Flag passati nelle tail call tra funzioni spezzate
- [x] Emissione in più file C e compilazione parallela (`altair image`): 357 MB di C in 242
  file, 49 secondi con clang `-O1` → `analisi/immagine.md`
- [x] Test differenziale sulla libreria dell'immagine intera, con clang e con gcc: 9.892
  funzioni identiche su 19.043 candidate, 0 diverse
- [x] Lettore `.forge` in Rust (`crates/forge`): contenitore e LZO1X, identico a `forge.py` su tre archivi

## Fase 2 · Runtime Win32 (gate raggiunto il 2026-11-15)

**Gate:** il gioco ricompilato parte, inizializza il CRT e apre i `.forge`. Stato in
`analisi/runtime.md`.

- [x] `crates/runtime` (`altair-run`): caricamento dell'immagine, IAT, TEB, TLS statica
- [x] Memoria guest, heap, `VirtualAlloc`
- [x] Thread, TLS, sezioni critiche, eventi, semafori, attese
- [x] File e percorsi (cartella del gioco in sola lettura, il resto in `run/c/`)
- [x] Registro (vuoto), file INI (solo valori predefiniti), `USER32` senza schermo, tempo
- [x] Avvio del CRT, inizializzatori statici, `WinMain`, 8 thread del motore
- [x] Oggetti COM come vtable di funzioni host
- [x] DirectSound muto, DirectInput e XInput senza dispositivi, Bink assente
- [x] **Apertura dei `.forge`** (il gate): `DataPC_Common`, `DataPC`, `DataPC_StreamedSounds`
- [x] Il gioco crea la finestra e chiama `Direct3DCreate9`: confine con la Fase 3
- [x] Lingua del gioco: registro emulato, `Language` = `Italian` (`ALTAIR_LANG`)
- [ ] SEH ed eccezioni C++; `WaitForMultipleObjects` con attesa di tutti
- [x] File INI veri (`Assassin.ini`, `DARE.INI` in `run/c/`)
- [x] Prima pagina dello spazio guest protetta e gestore degli accessi non validi

## Fase 3 · Grafica, audio, input

**Gate:** menu e tutorial nell'Animus giocabili.

- [x] `IDirect3D9` e device "registratore": il gioco presenta 3.000 fotogrammi; censimento
  in `analisi/d3d9.md` (79 metodi, solo shader model 3.0, DXT, 54 draw per fotogramma)
- [x] GPU da WSL: wgpu la raggiunge con il backend OpenGL e `GALLIUM_DRIVER=d3d12`; Vulkan è
  solo software (`crates/gpu-probe`, `analisi/d3d9.md`)
- [x] Triangolo fuori schermo sulla GPU (PNG verificato) e finestra via WSLg (212 fotogrammi in 4 s)
- [ ] Input simulato per superare la schermata iniziale e censire menu e gioco
- [x] Traduttore shader SM3 → WGSL (`crates/shader`): 1.488 shader del gioco tradotti e validati
- [x] D3D9 su wgpu, prima versione fuori schermo (`gfx.rs`): **il gioco disegna la schermata
  iniziale**, 74.341 draw senza errori → `analisi/d3d9.md`
- [x] Input simulato (`ALTAIR_KEYS`, `ALTAIR_TEXT`): schermata iniziale → profili → creazione
  del profilo (salvato su disco) → menu principale
- [x] `NEW`: il gioco carica il primo livello e **disegna il laboratorio Abstergo** (917 draw
  per fotogramma, nessun errore)
- [x] Interazione nel laboratorio (il gioco aspettava un tasto), schermata dell'Animus, ingresso
  nel ricordo: **Altaïr nel corridoio dell'Animus, con HUD e tutorial "Sistema di controllo"**;
  si muove con W (10 milioni di draw senza errori) → `analisi/runtime.md`, `run/scatti/`
- [x] Primi tre passi del tutorial superati con tastiera e mouse simulati (soggettiva,
  scostare i passanti, alto profilo): barra di sincronia da 1 a 4
- [ ] Completare il tutorial dell'Animus e arrivare a Masyaf: serve giocare davvero
- [x] Nomi dei tasti nei messaggi del tutorial: `EnumObjects` in UTF-16, tasti della tastiera compresi
- [x] Folla: gioco tradotto quasi due volte più veloce (`-O2`, `restrict`): da 9 a 12 fps
- [x] Invio alla GPU su un thread a parte: laboratorio 106 fps, elica del DNA 65, folla 20
  (erano 55, 30 e 12) → `analisi/d3d9.md`
- [x] Profilatore a campionamento nel runtime (`ALTAIR_PROFILE=1`)
- [ ] Folla: il costo per draw resta nel driver OpenGL-su-D3D12 di WSL; da rimisurare con
  Vulkan o D3D12 nativi. Leve rimaste: costanti legate una volta sola, registrazione dei draw
- [x] Canale di comando per pilotare il gioco mentre gira (`ALTAIR_CTL`, `tools/avvia.sh`, `tools/ctl.sh`)
- [x] Finestra vera (`ALTAIR_WINDOW=1`, `tools/gioca.sh`): si apre via Wayland e mostra il gioco
- [ ] Tastiera e mouse veri nella finestra: provata tutta la strada tranne il primo tratto
  (winit → stato); **manca la prova di una persona**
- [x] Stencil, scissor, depth bias, `ColorFill`, riciclo delle risorse
- [ ] Rettangoli parziali in `StretchRect`/`Clear` (mai visti finora), `GetRenderTargetData`
- [x] Prestazioni del renderer: da 2-25 a 27-190 fotogrammi al secondo secondo la scena
- [x] Video Bink veri (loghi e filmato introduttivo), con un **decodificatore nostro**
  (`crates/bink`): 3.381 fotogrammi identici a ffmpeg, audio entro 4 × 10⁻⁷ → `analisi/bink.md`
- [x] DirectSound su un mixer nostro: il gioco suona (verificato sul miscelato salvato)
- [x] Uscita audio su PulseAudio in tempo reale (100 s, zero blocchi scartati); **da ascoltare**.
  Di notte il server di WSLg era andato in crisi → `analisi/runtime.md`, sezione Audio
- [x] Diagnosi dell'input (`ALTAIR_INPUT_LOG`) e mouse sul canale di comando: la visuale ruota
- [ ] Suono 3D, EAX, XInput (controller)
- [ ] Confronto visivo con la DX10 originale, che sull'hardware attuale funziona: serve uno
  screenshot dell'utente della stessa scena (laboratorio Abstergo)
- [ ] (facoltativo, solo se una differenza di rendering non si chiarisce con la DX10) Far
  partire l'originale DX9, che oggi si blocca all'avvio: provare DXVK, dgVoodoo2, EaglePatch,
  modalità compatibilità, limite di core. Se parte: salvataggio e percorso fisso del tutorial
  come riferimento, ed eventuale traccia delle chiamate D3D9 con apitrace

## Fase 4 · Giocabile in nativo (gate principale)

**Gate:** partita completa, dall'inizio ai titoli di coda, salvataggi compresi.

- [ ] Registrazione e riproduzione delle partite come test di regressione
- [ ] Profilazione e correzione dei punti lenti (x87, chiamate indirette)
- [ ] Elenco dei bug noti chiuso

## Fase 5 · Browser

**Gate:** tutorial giocabile in Chrome.

- [ ] Profondità dello stack misurata in nativo prima di cominciare
- [ ] wasm32, guest a offset 0, gioco in Web Worker, WebGPU da OffscreenCanvas
- [ ] Dati in OPFS (scelta cartella con File System Access API), AudioWorklet
- [ ] Dimensione del modulo: streaming e cache della compilazione

## Fase 6 · Decompilazione mirata e mod

**Gate:** primo mod di gameplay funzionante.

- [ ] Sostituzione di funzioni guest per indirizzo (API di mod)
- [ ] Decompilazione di controller del giocatore, missioni, folla, sistema delle entità
- [ ] Poi i "nice to have" del documento di progetto, in ordine di rapporto valore/costo

---

## Prossimi passi concreti

1. **Fase 3, con una persona alla tastiera**: `tools/gioca.sh`, provare tastiera, mouse e audio
   nella finestra, completare il tutorial dell'Animus e annotare cosa non va.
   Poi: uscita audio affidabile, costo dell'invio dei draw, confronto visivo con la DX10.
2. Lettore `.forge` in Rust: fatto il contenitore; restano oggetti e classi.
3. Punti aperti della Fase 1: scenari mirati per le istruzioni non verificate, limiti della
   scoperta.
