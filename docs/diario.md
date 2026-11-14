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
- **Fase 1 aperta.** Workspace Rust (`crates/pe`, `crates/recomp`), binario `acrek` con
  `info`, `rtti`, `discover`, `translate`, `coverage`. Scoperta in 0,35 s: 116.090 funzioni.
- Traduttore x86 → C per le istruzioni intere e supporto C in `runtime/`. Si traduce l'85,9%
  delle funzioni (59,5% delle istruzioni); mancano `lock`, SSE, x87.
- `tools/difftest.py`: `strlen`, `strcmp`, `memcpy` tradotte identiche all'originale in Unicorn
  (1.717 chiamate). Il test ha trovato tre forme di jump table di `memcpy` non riconosciute,
  ora gestite → `analisi/traduttore.md`.
