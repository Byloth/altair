# Residui di sviluppo nell'eseguibile DX9

Domanda dell'utente (2026-11-15): prima di inventare un indicatore di FPS nostro, c'è nel
gioco qualcosa di già pronto (funzioni di sviluppo, comandi nascosti) da sfruttare?

Prima ricognizione, solo sulle stringhe e sui riferimenti nel codice. Tutto **verificato qui**
(script usa e getta in Python su `AssassinsCreed_Dx9.exe`, Capstone per il disassemblato), salvo
dove è scritto "ipotesi".

## Cosa c'è

**Un interprete di opzioni da riga di comando, completo.** Circa 60 opzioni confrontate una per
una con `stricmp` (`0xF9ED8E`) in una funzione che comincia intorno a `0x4089D0`, più il testo
di aiuto ("Command Line Options", `/help`). Tra le altre: `/windowed`, `/nosound`, `/nobink`,
`/nofps`, `/disablesave`, `/quit`, `/walkthrough`, `/walkthroughloading`, `/logtime`,
`/consoleout`, `/noconsole`, `/waitfordebugger`, `/preloadshaders`, `/remotepad`,
`/soundsystemdebug`, `/darespy`, una famiglia `/mem…` (statistiche di memoria) e opzioni da
strumento di produzione (`/computeao`, `/compresstextures`, `/buildloadinggrid`, `/startanvil`).

**`/nofps` implica un indicatore di FPS acceso di norma nelle build di sviluppo.** L'opzione
azzera il byte `+0x21EB` dell'oggetto delle opzioni; il costruttore (`0x408768`) lo inizializza;
`0x406341` lo copia nel byte `+0x92` dell'oggetto globale puntato da `0x1A0F058` (le
impostazioni del motore). Un secondo punto scrive lo stesso byte (`0x4077AC`).

**Classi di debug rimaste nell'RTTI:** `DebugCameraToggleEvent`, `GoDebugInputInterpreter`,
`DebugContext`, `DebugPath`, `IHumanDebug`, `QuadrupedDebug`, `SoundDebugOptions`,
`ShaderDebugInfoPack`, `LoadWorldMissionDebugHandler_m`, `QuickTeleportHandler` (con i suoi
pulsanti per mondo e portale: un menu di teletrasporto), `hkDebugDisplay` (Havok).

**Enumerazioni di visualizzazione:** `DisplayOptions::ProfilerStats` con 24 voci
(`ProfilerStatsCalls`, `…Memory`, `…Shadow0-3`, `…PostEffects`…), `DisplayOptions::FillMode`
(`WireFrame`), `RenderMode`, `LightingMode`, `ObjectSelectionMode`.

## Cosa manca

- **L'interprete delle opzioni è orfano.** Nessuna `call` diretta verso `0x4089D0`, nessun
  puntatore a quell'indirizzo nei dati; né l'interprete, né il costruttore delle opzioni, né
  `0x406341` sono tra le 117.106 funzioni che il traduttore raggiunge, e il gioco gira senza
  mai chiederle. Delle circa 3.000 istruzioni tra `0x408000` e `0x40AF1D` ne risultano
  raggiungibili 403. È codice rimasto nel binario ma scollegato.
- **Nessun testo per l'indicatore.** Tra le 43.067 stringhe (ASCII e UTF-16) non c'è alcun
  formato del tipo "FPS: %d" né simili. Ipotesi: il disegno del testo di debug è stato
  compilato via e ne è rimasto solo l'interruttore.
- **Nessun lettore trovato per il byte `+0x92` delle impostazioni** (seconda ricerca, stesso
  giorno). Nel codice tradotto nessuna funzione carica `[0x1A0F058]` e poi legge `+92h`; le
  16 letture di un byte a `+92h` in tutto il codice appartengono ad altre classi (guardate una
  per una quelle vicine al motore: `0xA07A46` è il calcolo del colore di una luce, `0xA05AB4`
  e `0xA05C1F` copiano una struttura, `0x9CF982` è codice di una libreria). L'oggetto nasce in
  `0x4066B9` (`call 0x9E5090`). Non è una prova definitiva, ma con l'assenza del testo basta
  per non contarci: l'indicatore è stato fatto nostro (`runtime.md`, "Pannello di diagnostica").

## Cosa se ne può fare

- Con un ricompilatore, ricollegare codice orfano è possibile: basta aggiungere l'indirizzo
  alle radici e chiamarlo. Ma un interruttore senza il codice che disegna non dà un
  indicatore di FPS: **per quello non c'è, a oggi, nulla di pronto da sfruttare.**
- Le piste che potrebbero dare qualcosa di funzionante, in ordine di promessa: il menu
  `QuickTeleportHandler` (è un gestore di menu come gli altri, quindi ha dati e codice), la
  telecamera di debug (`DebugCameraToggleEvent`), `FillMode` in fil di ferro, le statistiche
  del profiler. Per ognuna va verificato se il codice è raggiungibile e se i dati (`.forge`)
  contengono ancora gli oggetti. Non fatto.

Vedi `domande-aperte.md`.
