# Il runtime Win32 (`altair-run`)

Stato al 2026-11-15, primo giorno della Fase 2. Codice in `crates/runtime/`.

```sh
tools/build-image.sh          # rigenera l'immagine tradotta e ricompila solo ciò che è cambiato (circa 10 s)
tools/run.sh                  # avvia il gioco tradotto
tools/run.sh --trace          # con ogni chiamata a un import (oppure ALTAIR_TRACE=1)
```

## Come funziona

`altair-run` è un normale programma Linux:

1. controlla lo SHA-1 dell'eseguibile, lo legge e ne copia le sezioni in uno spazio guest di
   4 GB, agli indirizzi originali;
2. carica con `dlopen` la libreria prodotta da `altair image` (`build/libaltair_ac1-clang.so`);
3. riempie ogni cella della IAT con l'indirizzo di un trampolino (`0x7F000000 + 16·n`);
4. prepara TEB, PEB, TLS statica e stack del thread principale;
5. chiama l'entry point tradotto (`0x00F9F445`) su un thread host con 1 GB di stack.

Quando il codice tradotto chiama un indirizzo che non è una funzione tradotta, la libreria
passa a `host_call`, che cerca lo **shim** di quell'import: una funzione Rust che legge gli
argomenti dallo stack guest e scrive il risultato in `eax`. Se lo shim non esiste il gioco si
ferma dicendo quale import serve e da dove è stato chiamato. È il ciclo di lavoro della fase.

| Zona guest | Uso |
|---|---|
| `0x00100000`–`0x003F0000` | stack del thread principale |
| `0x00400000`… | l'immagine |
| `0x10000000`–`0x40000000` | heap (`HeapAlloc`) |
| `0x40000000`–`0x70000000` | `VirtualAlloc`, stack e TEB degli altri thread |
| `0x7F000000`… | trampolini degli import |
| `0x7FFD0000`, `0x7FFDF000` | TEB del thread principale, PEB |

| File | Contenuto |
|---|---|
| `main.rs` | caricamento e avvio |
| `machine.rs` | `Ctx` (identico a `runtime/altair.h`), accesso alla memoria guest, heap |
| `win32.rs` | meccanismo degli shim; KERNEL32: processo, memoria, TLS, sezioni critiche, attese, thread |
| `sync.rs` | eventi, semafori, thread come oggetti su cui aspettare |
| `fs.rs` | percorsi di Windows e API sui file |
| `com.rs` | oggetti COM: vtable in memoria guest i cui slot sono funzioni host |
| `dsound.rs` | DirectSound 8 muto |
| `input.rs` | DirectInput 8 e XInput senza dispositivi |
| `d3d9.rs` | Direct3D 9 "registratore": conta e non disegna (vedi `d3d9.md`) |
| `system.rs` | registro, file INI, data e ora, rete, Bink, numeri casuali |
| `user32.rs` | una finestra senza schermo |

Scelte fatte:

- **Shim di due tipi.** `L` riceve il runtime sotto un lucchetto globale; `F` non lo prende:
  serve ai percorsi caldi (`TlsGetValue`, sezioni critiche, `Interlocked*`) e a ciò che può
  restare in attesa (`WaitForSingleObject`).
- **Sezioni critiche senza stato sull'host**: il lucchetto è il campo `OwningThread` della
  struttura guest, preso con un compare-and-swap, con attesa a giri e poi brevi pause.
- **Thread guest = thread host**, ciascuno con il suo contesto, stack guest, TEB e TLS statica.
- **Heap** con i metadati sul lato host: un overflow nel gioco non li corrompe.
- **File system**: `C:\AC1\…` è la cartella del gioco, in **sola lettura** (una scrittura lì
  viene rifiutata e segnalata). Il resto di `C:\` vive in `run/c/` nel progetto, ignorata da
  git. Nomi senza distinzione tra maiuscole e minuscole.
- **Versione di Windows dichiarata**: XP SP3 (5.1.2600), un processore.
- **Oggetti COM**: `[vtable][riferimenti][dati]` in memoria guest; ogni interfaccia dichiara
  tutti i suoi metodi con il numero di argomenti, e quelli non implementati fermano il gioco
  dicendo quale serve ("METODO COM MANCANTE: IDirectSoundBuffer8::SetFX").
- **Audio**: un DirectSound che accetta tutto e non suona. I buffer esistono in memoria guest
  e il cursore di riproduzione avanza con l'orologio, così il mixer del gioco tiene il ritmo.
  EAX risulta assente.
- **Input**: tastiera e mouse esistono ma non vengono mai toccati; nessun controller.
- **Video**: `BinkOpen` fallisce, i filmati vengono saltati.
- **Memoria virtuale**: le zone di `VirtualAlloc` sono tracciate, perché il motore chiede a
  `VirtualQuery` quanto è grande una zona prima di liberarla.
- **Letture sovrapposte**: eseguite subito; l'evento di completamento viene segnalato.
- **Eccezioni**: nessun SEH. `RaiseException` gestisce solo l'eccezione con cui MSVC dà un nome
  ai thread; ogni altra ferma tutto.

## Dove arriva oggi

Con 234 shim di import e 87 metodi COM, senza alcuna modifica al codice tradotto:

1. **L'avvio del CRT** completo: heap, TLS, ambiente, locale, riga di comando.
2. **Tutti gli inizializzatori statici** del motore.
3. **`WinMain`**: Game Explorer di Vista (assente), hook della tastiera, cartelle speciali.
4. **I thread del motore**: `Loading`, `TaskThread1`, `Gear::AsynchDevice` e altri, 8 in tutto.
5. **L'audio**: `EAXDirectSoundCreate8` fallisce, `DirectSoundCreate8` riesce; il thread del
   mixer gira e scrive nei buffer (circa 90 `Lock`/`Unlock` al secondo).
6. **I `.forge`**: apre `DataPC_Common.forge`, `DataPC.forge` e `DataPC_StreamedSounds.forge`
   con letture sovrapposte, e li legge dal thread di caricamento. **È il gate della fase.**
7. **L'input**: crea DirectInput, mouse e tastiera, e li interroga dal ciclo principale.
8. **La finestra**: registra la classe `ScimitarEngineWindowClass` (procedura `0x004013E3`) e
   crea la finestra "Assassin's Creed" 1280x720, da un thread del motore.
9. **Direct3D**: crea il device (1280x720, D24S8), 3.412 shader, texture e render target.
10. **Il ciclo principale**: carica la stanza di caricamento e il mondo del menu, salta i
    filmati iniziali e **presenta fotogrammi**: 3.000 di fila, con circa 54 draw ciascuno,
    senza fermarsi. Il thread del mixer audio usa il codice MMX tradotto.

Il gioco non si ferma più da solo: gira finché non lo si interrompe (`ALTAIR_FRAMES=N`).
Non disegna nulla: Direct3D è un registratore.

Cose notate lungo la strada:
- Il gioco cerca `DataPC_StreamedSoundsEng.forge`, che in questa installazione non esiste
  (c'è solo quello italiano): dichiariamo la lingua inglese. Da sistemare con la lingua vera.
- Apre `videos\LoadingScreen.bik` direttamente come file, oltre che tramite Bink.
- Senza dispositivo audio il gioco mostra "Cannot initialize Sound Driver" e poi chiama un
  oggetto nullo: l'originale non parte senza scheda audio.

## Cosa ha insegnato

- **Una funzione che la scoperta non vedeva.** La prima chiamata indiretta fallita è stata verso
  `0x00FB2451`, nelle routine matematiche del CRT: ci si arriva da una tabella di puntatori in
  `.data` **non allineata a 4** (a `0x019CB64A`), come aveva notato scimitar-recomp. La scoperta
  ora cerca in `.data` anche le parole non allineate che puntano a codice subito dopo un `ret`
  o un `jmp`: 65 partenze in più (116.142 funzioni). Alcune saranno falsi positivi: costano
  solo un po' di codice in più.
- **I puntatori nulli non fanno crash.** Lo spazio guest è mappato per intero, quindi leggere
  dall'indirizzo 0 dà 0 invece di un'eccezione. Un oggetto nullo diventa una chiamata
  all'indirizzo 0, che il runtime segnala: comodo, ma l'errore emerge più tardi del dovuto.
  Da valutare: proteggere la prima pagina.
- La ricompilazione incrementale funziona: dopo la modifica alla scoperta sono stati
  riscritti 26 file su 242, e tutto è stato ricompilato in 10 secondi.

- **`VirtualQuery` conta.** Con una risposta fissa ("una pagina") il motore, liberando un
  blocco, calcolava male a quale allocatore appartenesse e chiamava un oggetto inesistente.
  Trovarlo ha richiesto la traccia degli import e una nuova diagnostica: a ogni arresto il
  runtime stampa le ultime 32 destinazioni di chiamate indirette del thread (`altair_trail`),
  che mostrano il percorso anche attraverso le tail call.
- **Letture sovrapposte.** Il thread di caricamento aspettava per sempre l'evento di
  completamento di `ReadFile`: va segnalato anche se la lettura è già finita.

- **Altre funzioni che la scoperta non vedeva**, emerse solo eseguendo:
  - una seconda etichetta del CRT in una tabella non allineata: ora si riconoscono le *serie*
    di tre o più puntatori a codice a indirizzi non allineati;
  - una funzione chiamata con una `call` diretta ma raggiunta dal traduttore e non dalla
    scoperta: `altair image` ora aggiunge ogni destinazione di `call` che manca (93 funzioni);
  - una callback che è solo `ret 8`, la coda di un'altra funzione; e la sua vicina di tabella,
    nascosta da due **falsi inizi** nati da costanti numeriche. La scoperta ora elabora i dati
    prima degli immediati, richiede due `nop` (un solo `0x90` è spesso un byte ModRM) e accetta
    le voci delle tabelle di puntatori solo in una seconda passata;
  - un mixer audio MMX scartato perché conteneva `pushad`, che la scoperta considerava
    "istruzione che un gioco non esegue".
  In tutto le funzioni sono passate da 116.142 a **117.159**.
- **Callback verso il gioco**: `EnumDisplayMonitors` chiama una funzione guest. `call_guest`
  lo permette agli shim che non tengono il lucchetto del runtime.

## Prossimi passi

1. **Vedere qualcosa.** Verificare se wgpu trova la GPU da WSL; poi il renderer vero, partendo
   da ciò che questa scena usa: shader model 3.0 → WGSL, DXT, render target A8R8G8B8 e R32F.
2. Input simulato (un tasto premuto) per superare la schermata iniziale e censire i menu.
3. Riciclo degli oggetti COM rilasciati: oggi non si libera nulla.
4. Punti aperti: SEH, file INI veri, lingua, prima pagina protetta.
