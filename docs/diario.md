# Diario

## 2026-11-14

- Letto il documento di progetto "AC1 Recompiled: piano di progetto".
- Installata la build Steam in `E:\Giochi\Steam\steamapps\common\Assassins Creed`.
  La DX9 si blocca all'avvio; la DX10 funziona.
- Verificato lo SHA-1 della DX9 (`a9ea476e…432120`): coincide con quello che EaglePatch
  indica per Steam e GOG.
- Confrontate DX9 e DX10: stesso changelist 86610, DX9 con 4.043 nomi RTTI contro 937 (conteggio poi corretto: 4.376 contro 939).
  → `analisi/eseguibili.md`, `decisioni.md`.
- Creati la struttura del repository, `tools/pe.py` e il piano di lavoro (`PIANO.md`).
- `git init`; clonati scimitar-recomp (`4f36655`) ed EaglePatch (`a39dc87`) in `ref/`, ignorata da git.
  Correzione: niente più cartelle fuori dal progetto.
- Letti `AC1_WINDOWS.md` e `PLAN.md` di scimitar-recomp → `analisi/scimitar-recomp.md`.
- **Scoperta:** la build Ubisoft Connect di scimitar-recomp ha lo stesso layout della nostra
  (entry, sezioni, TLS `0x1A3561C`, cookie `0x19CA838`, 178 export). I loro indirizzi valgono
  per noi → `decisioni.md`.
- Raccolti gli indirizzi DX9 di EaglePatch → `analisi/indirizzi-noti.md`.
- Toolchain da installare a cura dell'utente: Rust, clang/lld, build-essential, python3-venv,
  JDK 21, Ghidra 12.2 in `opt/`.
- Verificato l'ambiente: clang 18, lld, cmake, Java 21 presenti; Rust non trovato in WSL.
  Creato `.venv/` (unicorn 2.1.4, capstone 5.0.7). Scaricato Ghidra 12.2 in `opt/`
  (SHA-256 verificato). Ghidra 12.2 richiede **Java 25**, non 21 → `ambiente.md`.
- Rust trovato: 1.100.0 in `~/.local/share/cargo` (PATH in `~/.bash_exports`, non in `~/.cargo`).
  Java 25 ancora da installare.
- Installato Java 25 (rimosso il 21). Ghidra headless parte. Ambiente della 0.2 completo.
- Completata la 0.3: letti anche `PC_DATA.md`, `ACR_WINDOWS.md`, `ANVIL_FAMILY.md`, parti di
  `METAL.md` e `README.md`, `recomp/README.md`, `platform/README.md`. Aggiunti ad
  `analisi/scimitar-recomp.md`: formato `.forge` di AC1 (v25, LZO1X_999), 8 bug D3D9 condivisi,
  insidie del runtime Win32 (QPC a 10 MHz, race multi-CPU), mappa del loro layer Windows.
- **0.5 chiusa.** `tools/rtti.py`: 4.376 type descriptor, 4.597 vtable, 59.899 slot, 28.717
  funzioni, identici a scimitar-recomp → `analisi/rtti.md`. Corretta la regex di `pe.py rtti`
  (troncava i template). `_purecall` è a `0xF9F7A2`.
- RTTI della DX10: nessuna classe `scimitar`, solo librerie; 5.059 coppie di funzioni
  DX9 ↔ DX10 dalle vtable omonime → `analisi/rtti.md`.
- **0.6 chiusa.** `tools/imports.py` → `analisi/import.md`: 260 import tutti referenziati;
  `TlsGetValue` ha 2.644 riferimenti, `DebugBreak` 144 (assert). GUID COM di DirectInput 8,
  DirectSound 8/3D ed EAX 2–4 trovati in `.rdata`. Metodi COM rimandati al runtime.
- **0.7 chiusa (prototipo).** `tools/functions.py`: 115.807 partenze, 3,19 M di istruzioni
  (96.781 con `--strict`; scimitar-recomp 98.470). Dall'entry per chiamate dirette si
  raggiungono solo 7.216 funzioni: il resto passa da vtable e puntatori. 3,7 MB di `.text`
  sono codice non referenziato → `analisi/funzioni.md`. Import in Ghidra rimandato.
- **0.8 chiusa.** `tools/harness.py`: `strlen`, `strcmp`, `memcpy` dell'originale eseguiti in
  Unicorn e verificati; `fs:` impostato tramite GDT (Unicorn a 32 bit non accetta FS_BASE).
  Gate della Fase 0 soddisfatto → `analisi/harness.md`.
- Spostata la vecchia 0.9 (originale DX9 su Windows) in Fase 3 come attività facoltativa → `decisioni.md`.
- **0.10 chiusa, Fase 0 chiusa.** `tools/forge.py` legge i 18 archivi (15.906 voci). Ogni
  oggetto porta il CRC-32 (standard) del nome della sua classe: incrociato con l'RTTI, il 100%
  degli oggetti è riconosciuto. Le missioni sono oggetti `MissionItem*` nei dati (27 classi,
  circa 950 oggetti in `DataPC.forge`) → `analisi/forge.md`.
- **Fase 1 aperta.** Workspace Rust (`crates/pe`, `crates/recomp`), binario `altair` con
  `info`, `rtti`, `discover`, `translate`, `coverage`. Scoperta in 0,35 s: 116.090 funzioni.
- Traduttore x86 → C per le istruzioni intere e supporto C in `runtime/`. Si traduce l'85,9%
  delle funzioni (59,5% delle istruzioni); mancano `lock`, SSE, x87.
- `tools/difftest.py`: `strlen`, `strcmp`, `memcpy` tradotte identiche all'originale in Unicorn
  (1.717 chiamate). Il test ha trovato tre forme di jump table di `memcpy` non riconosciute,
  ora gestite → `analisi/traduttore.md`.
- `altair candidates` + `difftest.py --auto`: test differenziale su funzioni scelte dal
  ricompilatore, con ingressi generati. Le prime 6 differenze erano memoria residua tra i
  casi, non bug: ora ogni caso riparte da heap (1 MB), stack e dati globali ripristinati.
- Aggiunte atomiche (`lock`, `xchg` con memoria), `xadd`, `cmpxchg`, `shld`/`shrd`,
  `rcl`/`rcr`, `bts`/`btr`/`btc`, `bsf`/`bsr`, `pushfd`/`popfd`, `sahf`/`lahf`, `mul`/`div` a
  8 e 16 bit, `rdtsc`, `cpuid`. Copertura: 89,5% delle funzioni, 65,1% delle istruzioni.
- Campione di 4.000 funzioni: **1.753 identiche all'originale, 0 diverse**, 2.247 saltate.
  Restano solo SSE e x87 → `analisi/traduttore.md`.

## 2026-11-15

- `altair census`: istruzioni per tipo, con quante non traducibili.
- **SSE/SSE2** nel traduttore (`runtime/altair_sse.h`, in C portabile) e stato XMM nel contesto.
  Copertura: 94,3% delle funzioni; non traducibile solo il 2,9% delle istruzioni (x87, MMX).
- Il test confronta anche i registri XMM, con ingressi float (zeri con segno, infiniti, NaN,
  denormali). Campione di 6.000 funzioni: **3.042 identiche, 0 diverse**.
- **Trovato un bug di clang 18.1.3** sulle scritture dei registri parziali; riprodotto in
  `tests/compiler/clang18-partial-register.c` e aggirato con scritture strette.
- Differenze di NaN tra hardware ed emulatore (regola x87 applicata a SSE): tollerate e contate.
- L'emulatore si portava dietro flag di direzione e MXCSR tra un caso e l'altro: corretto
  nell'harness → `analisi/traduttore.md`, `analisi/harness.md`.
- **Rinominato il progetto tecnico da `acrek` ad `altair`** (binario, crate, file e prefissi
  del runtime, documenti). Le voci di diario precedenti sono state aggiornate al nuovo nome.
  Test rifatti dopo il cambio: selftest e scenari OK, 820 funzioni identiche su 1.500.
  La cartella va rinominata dall'utente a sessione chiusa → `ambiente.md`.
- Titolo esteso del progetto cambiato in "AC1 Recompiled - Altaïr" (repository e documento online) → `decisioni.md`.
- **x87** nel traduttore (`runtime/altair_x87.h`): registri `long double` a 80 bit, parola di
  controllo dell'host allineata a quella del guest. Copertura: **116.048 funzioni su 116.090
  (99,96%)**, non traducibili 336 istruzioni su 3,2 milioni.
- Il test confronta anche lo stato x87. Campione di 8.000 funzioni: **4.201 identiche, 0
  diverse** (6 a meno di un NaN). Corretto un caso: codice che dopo una `call` prosegue dentro
  un'altra funzione (routine matematiche del CRT) → `analisi/traduttore.md`, `decisioni.md`.
- **Le ultime 42 funzioni.** 17 erano dati (tabelle di indici degli switch, costanti tonde):
  ora la scoperta le scarta. 25 erano codice vero: MMX dei mixer audio, registri di segmento,
  `xlatb`, `fnstenv`/`fnsave`, `bts` su stringa di bit e poche altre. **Ora si traducono tutte
  le 116.077 funzioni.** Verifica delle 25: 17 identiche, 7 saltate, 1 diversa solo nei campi
  non modellati di `fnstenv`. Campione da 8.000: 4.199 identiche, 0 diverse.
  `altair coverage --list`, `difftest.py --funcs` → `analisi/traduttore.md`.
- **`altair image`: l'immagine intera.** 116.077 funzioni in 242 file C (uno per finestra di
  64 KB di indirizzi, 357 MB), generati in 3 secondi. Compilazione: 49 s con clang `-O1`, 55 s
  con `-O2`, 50 s con gcc. Test sulla libreria intera: 9.892 funzioni identiche su 19.043
  candidate, 0 diverse, con entrambi i compilatori. **Gate della Fase 1 raggiunto** →
  `analisi/immagine.md`. (Primo tentativo compilato per sbaglio con gcc: `CC ?=` nel Makefile
  non scavalca il `cc` predefinito di make.)
- **Fase 2 aperta: `altair-run`.** Runtime Win32 in Rust (`crates/runtime`), 170 shim. Al primo
  avvio il gioco tradotto ha eseguito l'intero avvio del CRT (588 chiamate a import) fermandosi
  su `CreateEventA`. Con thread, eventi, file, registro, INI e una finestra senza schermo
  arriva a: inizializzatori statici, `WinMain`, 8 thread del motore (`Loading`,
  `TaskThread1`…), inizializzazione dell'audio.
- Si ferma sull'audio: senza dispositivo il gioco mostra "Cannot initialize Sound Driver" e
  chiama un oggetto nullo. Serve un DirectSound muto. Nessun `.forge` aperto finora.
- Trovata dal runtime una funzione non scoperta (`0xFB2451`, tabella non allineata in `.data`):
  la scoperta ora la vede, +65 funzioni. Ricompilazione incrementale: 26 file, 10 secondi.
  Test differenziale dopo la modifica: 2.110 identiche su 4.000, 0 diverse → `analisi/runtime.md`.
- **Oggetti COM** (`com.rs`) e sopra di essi **DirectSound muto**, DirectInput e XInput senza
  dispositivi, Bink assente. Il gioco supera l'audio e **apre i `.forge`: gate della Fase 2**.
- Due blocchi risolti con la traccia: le letture sovrapposte (evento di completamento mai
  segnalato) e `VirtualQuery` (risposta fissa → il motore sbagliava allocatore e chiamava un
  oggetto nullo). Nuova diagnostica: `altair_trail`, le ultime destinazioni indirette.
- Il gioco ora crea la finestra "Assassin's Creed" e chiama **`Direct3DCreate9`**: lì si ferma,
  tre corse su tre. È il confine con la Fase 3. Test differenziale: 1.616 identiche su 3.000,
  0 diverse → `analisi/runtime.md`.
- **Direct3D 9 registratore** (`d3d9.rs`): 17 + 119 metodi dichiarati, più risorse e tabella
  delle costanti D3DX. **Il gioco gira nel suo ciclo principale e presenta 3.000 fotogrammi**
  (54 draw l'uno) senza fermarsi: carica stanza di caricamento e mondo del menu, salta i filmati.
  Censimento in `analisi/d3d9.md`: 79 metodi usati, 3.412 shader tutti SM 3.0, texture DXT.
- Cinque funzioni non scoperte emerse eseguendo, e la scoperta migliorata di conseguenza
  (serie non allineate, rete di sicurezza sulle `call`, callback di solo `ret`, falsi inizi da
  immediati, `pushad`): 117.159 funzioni. `call_guest` per le callback verso il gioco.
- GPU: `/dev/dxg` presente, RTX 4070 Ti visibile da WSL; wgpu ancora da provare.
- Test differenziale: 3.084 identiche su 6.000, 0 diverse.
- GPU in WSL: wgpu vede la RTX 4070 Ti tramite OpenGL su D3D12 (`GALLIUM_DRIVER=d3d12`); Vulkan solo software. Nessuna installazione necessaria. `crates/gpu-probe` → `analisi/d3d9.md`, `ambiente.md`.
- Triangolo con wgpu sulla RTX 4070 Ti da WSL: fuori schermo (PNG controllato) e in finestra via WSLg (212 fotogrammi in 4 s). Core dump alla chiusura della finestra, da sistemare.
