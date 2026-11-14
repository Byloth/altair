# Piano di lavoro

Versione operativa del documento di progetto
([AC1 Recompiled: piano di progetto](https://claude.ai/artifact/SoPWUhJZPJ45oWk3t4r8bu)).
Ogni fase si chiude con un **gate** verificabile; non si passa alla fase successiva senza.
Spunta le caselle man mano; ogni risultato va scritto in `analisi/`, `decisioni.md` o `diario.md`.

```
0 Fondamenta → 1 Ricompilatore → 2 Runtime Win32 → 3 Grafica/audio/input
  → 4 GIOCABILE IN NATIVO (gate principale) → 5 Browser → 6 Decompilazione mirata e mod
```

---

## Fase 0 · Fondamenta (in corso)

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

### 0.7 Primo elenco di funzioni

- [ ] Partenze forti: entry point, export, vtable, puntatori a funzione in `.rdata`
- [ ] Partenze deboli: prologhi MSVC tipici dopo padding `int3`/`nop`
- [ ] Confronto con il conteggio di scimitar-recomp (~98.000 funzioni) → `analisi/funzioni.md`
- [ ] Importare RTTI e funzioni in Ghidra per avere una vista navigabile

### 0.8 Harness Unicorn

- [ ] `tools/harness.py`: mappa l'immagine a `0x400000`, prepara stack, TEB e `fs:`, intercetta
  gli import con stub
- [ ] Eseguire una funzione foglia del gioco (es. una routine di stringhe o math del CRT)
  e confrontarne il risultato
- [ ] Documentare in `analisi/harness.md` come scrivere un test differenziale

### 0.9 L'originale DX9 in esecuzione (non bloccante)

- [ ] Riprodurre il blocco della DX9 e annotare i sintomi
- [ ] Provare DXVK, dgVoodoo2, EaglePatch, compatibilità Windows, limite di core
- [ ] Se parte: preparare un salvataggio e un percorso fisso del tutorial come riferimento visivo

### 0.10 Dati del gioco (ricognizione)

- [ ] Inventario dei `.forge` (17 archivi, 6,8 GB): header, indice, tipi di file contenuti
  → `analisi/forge.md`
- [ ] Prima risposta alla domanda "missioni: dati o codice?" (guardando i tipi negli archivi)

---

## Fase 1 · Ricompilatore

**Gate:** tutto `.text` tradotto in C che compila; test differenziali verdi su un campione
ampio di funzioni.

- [ ] Workspace Rust: `crates/pe`, `crates/recomp`, `crates/runtime`
- [ ] Parser PE e loader dell'immagine (riuso del lavoro della 0.5–0.7)
- [ ] Scoperta delle funzioni: partenze forti e deboli, jump table MSVC, code prima dell'entry
- [ ] Traduzione delle istruzioni x86 in C: interi, flag lazy, SSE, x87 a 80 bit
- [ ] Accessi a `fs:` (TEB), chiamate indirette tramite tabella delle funzioni tradotte
- [ ] Emissione in più file C per compilazione parallela
- [ ] Test differenziali automatici contro l'harness Unicorn
- [ ] Copertura: percentuale di funzioni tradotte e verificate → `analisi/copertura.md`

## Fase 2 · Runtime Win32

**Gate:** il gioco ricompilato parte, inizializza il CRT e apre i `.forge`.

- [ ] Memoria guest (2 GB, immagine a `0x400000`), heap, `VirtualAlloc`
- [ ] Thread, TEB, TLS, SEH, sincronizzazione
- [ ] File e percorsi (cartella del gioco in sola lettura, salvataggi in una cartella nostra)
- [ ] Registro emulato, `USER32` minimo (finestra, messaggi), tempo
- [ ] Oggetti COM come vtable di funzioni host
- [ ] Stub tracciati per tutto ciò che manca, con log della prima chiamata

## Fase 3 · Grafica, audio, input

**Gate:** menu e tutorial nell'Animus giocabili.

- [ ] D3D9 su wgpu: device, stati, risorse, formati, render target
- [ ] Traduttore shader SM2/SM3 → WGSL
- [ ] Video Bink, DirectSound (EAX opzionale), DirectInput, XInput
- [ ] Confronto visivo con l'originale (se la 0.9 riesce) o con scimitar-recomp

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

1. **0.7**: primo elenco di funzioni (partenze forti e deboli).
2. **0.8**: harness Unicorn.
3. **0.10**: ricognizione dei `.forge`.
