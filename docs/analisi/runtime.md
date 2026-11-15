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

## Input simulato

Finché non c'è una finestra vera, i tasti arrivano da un copione legato ai fotogrammi:

```sh
ALTAIR_KEYS="900:28,1500:28,2300:28"   # fotogramma:codice DIK[:durata in fotogrammi]
ALTAIR_TEXT="2000:Matteo"              # fotogramma:testo, un carattere ogni 12 fotogrammi
```

Il gioco legge la tastiera in due modi diversi, entrambi serviti:

- **Navigazione** (Invio, frecce, "premi un tasto"): stato immediato di DirectInput,
  `GetDeviceState` sulla tastiera, 256 byte indicizzati dai codici DIK (28 Invio, 1 Esc,
  57 Spazio, 200/208/203/205 frecce).
- **Testo** (nome del profilo): messaggi della finestra. Il thread che possiede la finestra
  chiama `PeekMessageA` a ogni fotogramma, poi `TranslateMessage` e `DispatchMessageA`; la
  procedura della finestra (`0x004013E3`, codice del gioco richiamato con `call_guest`) su
  `WM_KEYDOWN` chiede il carattere con `GetKeyboardState` e `ToUnicode`. `WM_CHAR` non serve.

Con il copione qui sopra il gioco **crea un profilo**: scrive
`C:\Users\Player\AppData\Roaming\Ubisoft\Assassin's Creed\Saved Games\<numero>.hdr` e
`.sav`, cioè `run/c/Users/Player/…` nel progetto, e alla corsa successiva lo ritrova. Cerca
anche un `<numero>.opt` (le opzioni), che non esiste ancora.

Le frecce servono anche come messaggi `WM_KEYDOWN` (gli elenchi dei menu le leggono da lì); il
copione le manda in entrambi i modi. I tempi sono in fotogrammi, ma il gioco ragiona in tempo
reale: un tasto premuto mentre un menu sta ancora comparendo viene ignorato. Il copione che
porta al laboratorio Abstergo con il profilo "Claude" (secondo dell'elenco):

```sh
ALTAIR_KEYS="900:28,1500:208:6,1700:208:6,1900:28,3300:28,3900:28:30,4500:28:3"
```

### Canale di comando (dal 2026-11-15)

I copioni a fotogrammi restano, ma per esplorare si usa il canale di comando: con
`ALTAIR_CTL=cartella` il runtime, a ogni `Present`, legge e cancella `cartella/cmd` (una riga
per comando) e scrive in `cartella/stato` il numero del fotogramma.

| Comando | Effetto |
|---|---|
| `key DIK [fotogrammi]` | preme un tasto (predefinito 10 fotogrammi) |
| `text TESTO` | batte un testo come messaggi della finestra |
| `mouse DX DY [ROTELLA]` | sposta il mouse (relativo, come lo legge DirectInput) |
| `button N [fotogrammi]` | preme un pulsante del mouse (0 sinistro, 1 destro, 2 centrale) |
| `move X Y`, `click X Y` | porta il puntatore in un punto dello schermo, e clic sinistro |
| `stats` | stampa il ritmo dell'ultimo intervallo |
| `shot FILE.png` | salva il back buffer in `cartella/FILE.png` |
| `quit` | stampa il censimento ed esce |

```sh
tools/avvia.sh --partita          # avvia in background e arriva alla partita del profilo "Claude"
tools/avvia.sh --animus           # prosegue fino al corridoio dell'Animus con Altaïr (5 minuti)
tools/ctl.sh "key 28" "shot a.png"
tools/ctl.sh quit
```

## Dal laboratorio all'Animus (2026-11-15)

Percorso verificato con il canale di comando, profilo "Claude", lingua italiana:

1. `CONTINUA` dal menu principale carica `DataPC_Present_Room.forge`: scena nel laboratorio
   Abstergo, Vidic parla a Desmond seduto sull'Animus (circa due minuti).
2. Compare "PREMERE QUALUNQUE PULSANTE PER INTERAGIRE" e **la scena aspetta**: senza un tasto
   non prosegue (verificato: ferma per oltre cinque minuti). Con Spazio prosegue.
3. Schermata dell'Animus vista dal lettino: "CRONOLOGIA MEMORIA GENETICA", l'elica del DNA,
   "Ricordo bloccato", i volti di Lucy e Vidic; poi "IN ATTESA DI INPUT". Un altro tasto, e le
   frecce spostano il puntatore sull'elica.
4. Il gioco apre `DataPC_Memory_Corridor.forge` e `DataPC_extra.forge`: **Altaïr nel corridoio
   dell'Animus**, con la barra di sincronizzazione; parte il tutorial "SISTEMA DI CONTROLLO"
   (Testa, Mano nuda, Mano armata, Gambe) e compaiono due personaggi. Con W Altaïr cammina.

Schermate in `run/scatti/` (ignorata da git: sono immagini del gioco). In tutta la sessione,
10 milioni di draw, nessun draw saltato e nessun errore di validazione.

Non ancora fatto: completare i passi del tutorial (richiedono combinazioni di tasti e mouse).

## Registro e lingua

Verificato con `ALTAIR_TRACE`: il gioco apre una sola chiave, `HKLM\SOFTWARE\Ubisoft\Assassin's Creed`
(da `0x00402289`), e ne legge il valore `Language` (da `0x004022AD`). I nomi accettati sono
nell'eseguibile accanto alla chiave (`0x1680D24` e seguenti): `English`, `French`, `Spanish`,
`German`, `Italian`, `Japanese`, `Korean`, `Russian`, `Czech`, `Polish`. Senza la chiave il
gioco parte in inglese e cerca `DataPC_StreamedSoundsEng.forge`, che nell'installazione
italiana non c'è.

`system.rs` emula quella chiave: `Language` vale `ALTAIR_LANG` (predefinito `Italian`),
`InstallDir` vale `C:\AC1\`. Con `Italian` il gioco apre `DataPC_StreamedSoundsIta.forge` e
mostra i testi in italiano.

## Audio

`dsound.rs`. Il gioco (motore audio DARE) crea decine di buffer secondari da 38.400 byte,
48 kHz, con `Play` in ciclo, e li riempie man mano seguendo il cursore di riproduzione: sono
flussi, la miscelazione finale la fa DirectSound. Quindi:

- un thread `mixer` ogni 10 ms somma 480 fotogrammi di ogni buffer che suona (8/16 bit, mono o
  stereo, ricampionamento lineare se la frequenza non è 48 kHz, volume e bilanciamento in
  centesimi di decibel) ed è **lui a far avanzare i cursori**: il gioco tiene il ritmo anche
  senza uscita;
- i buffer hanno un lock loro: quello del runtime è quasi sempre in mano al gioco;
- un thread `audio-out` scrive su PulseAudio con `pa_simple_write` di `libpulse-simple.so.0`,
  caricata con `dlopen` (presente in WSL, niente da installare, niente header). Tra i due c'è
  una coda di 60 ms: se l'uscita non consuma, i blocchi si scartano e si contano;
- `ALTAIR_AUDIO=0` spegne l'uscita; `ALTAIR_AUDIO_DUMP=file.raw` salva il miscelato
  (S16LE, 48 kHz, stereo).

Misurato: nel menu e nel laboratorio il miscelato ha picchi tra 15.000 e 30.000 su 32.767 per
tutti i 400 secondi di una sessione, quindi il gioco decodifica e suona davvero (musica, voci,
ambiente). Non fatto: suono 3D (posizione, distanza), EAX.

**L'uscita su PulseAudio funziona in tempo reale con la coda da 200 ms; non è ancora stata
ascoltata da una persona.** Misura del 2026-11-15 mattina, a server tornato in salute: 100
secondi di gioco (loghi, filmato introduttivo con il suo audio), 100,8 secondi di miscelato,
**zero blocchi scartati**.

Storia, perché il problema può tornare. Di notte l'uscita accettava un quinto del tempo reale,
rallentava anche gli altri programmi (`ffmpeg -f pulse`) e alla fine il server ha smesso di
rispondere (`pa_simple_new`: errore 8, timeout) per più di un'ora; poi si è ripreso da solo.
La prima versione chiedeva una coda di 60 ms: l'ipotesi, coerente con le misure ma non
dimostrata, è che una coda così corta mandi in crisi il sink RDP di WSLg per tutti, e che le
prove successive con 200 ms siano state fatte a server già compromesso. Se ricapita:
`ALTAIR_AUDIO=0` per giocare senza, `wsl --shutdown` da Windows per riavviare il server.

`tools/audio-probe.py` misura l'uscita fuori dal gioco: suona 4 secondi di tono a blocchi, in
tempo reale come il mixer, e dice quanto ci ha messo. Con il server in salute: `pa_simple` con
coda predefinita o da 200 ms, e SDL2 (`libSDL2`, presente in WSL, che usa PulseAudio a modo
suo), consumano tutti 4 secondi in 4,0-4,2. SDL2 resta l'alternativa pronta se `pa_simple`
tornasse a dare problemi.

## Video Bink

`bink.rs`. Il gioco importa 15 funzioni di `binkw32.dll` e riproduce cinque filmati da
`videos\`: `LoadingScreen` (in ciclo durante i caricamenti, 15 fps, muto), `UBI_LOGO`,
`Ubisoft_Presents`, `Warning_Disclaimer` e `DemoIntro` (29,97 fps, 3.020 fotogrammi, quattro
tracce audio: fronte, centro, bassi, retro). Tutti 1280×720.

Sequenza osservata per fotogramma (conteggi stampati alla chiusura di ogni filmato):
`BinkWait` finché non è ora → `BinkDoFrame` → `BinkCopyToBuffer` (formato 32 bit, dentro una
texture bloccata) → `BinkShouldSkip` → `BinkNextFrame`. `BinkGetFrameBuffersInfo` è chiamata una
volta sola per filmato. Della struttura `BINK` bastano i primi campi (larghezza, altezza,
fotogrammi, fotogramma corrente, cadenza).

Non c'è la libreria di RAD: l'intestazione del file dà dimensioni e cadenza, e i fotogrammi li
decodifica **`ffmpeg`** (già installato in WSL), avviato come processo con uscita `rawvideo
yuv420p` su una pipe; la conversione in BGRA è nostra (BT.601). L'audio lo decodifica un
secondo `ffmpeg` (somma di fronte, centro e retro in stereo a 48 kHz) e finisce nel mixer.
Senza `ffmpeg`, o con `ALTAIR_VIDEO=0`, `BinkOpen` fallisce e il gioco salta il filmato.
Invio salta il filmato introduttivo.

## Finestra vera

`window.rs`, con `ALTAIR_WINDOW=1`. Il thread principale del runtime, che altrimenti aspetta
soltanto la fine del gioco, esegue il ciclo degli eventi di winit; il renderer crea la
superficie wgpu sulla finestra e a ogni `Present` ci copia il back buffer, scalato alla
dimensione della finestra. Gli eventi si accumulano in uno stato condiviso:

- tasti → tabella dei codici DIK per `GetDeviceState` della tastiera, e messaggi
  `WM_KEYDOWN`/`WM_KEYUP` con il carattere pronto per `ToUnicode`;
- mouse → `DIMOUSESTATE` (spostamento dall'ultima lettura, rotella, pulsanti), `GetCursorPos`
  e messaggi `WM_MOUSEMOVE`, `WM_xBUTTONDOWN/UP`;
- chiusura della finestra → uscita immediata del processo;
- **F10** cattura o libera il puntatore (non arriva al gioco): catturato, il mouse muove la
  visuale senza fermarsi ai bordi; libero, serve per i menu.

Senza la variabile tutto resta fuori schermo, come prima. `tools/gioca.sh` la imposta.

### Diagnosi dell'input: `ALTAIR_INPUT_LOG=1`

Una riga per ogni passaggio di un evento: `finestra:` (winit l'ha ricevuto), `messaggio:`
(consegnato alla procedura della finestra), `DirectInput tastiera/mouse:` (il gioco l'ha letto
con `GetDeviceState`; stampato solo quando cambia). Se un tasto non fa nulla, il registro
dice a quale passaggio si è perso.

I comandi `mouse`, `button`, `move`, `click` del canale di comando scrivono nello stesso stato
che riempie la finestra, quindi provano la stessa strada. Verificato così (2026-11-15): nel
corridoio dell'Animus `mouse 400 0` **ruota la visuale** attorno ad Altaïr, e il clic arriva
sia a DirectInput sia come `WM_LBUTTONDOWN`. Resta non provato solo il primo tratto, da winit
allo stato.

Tasti predefiniti del gioco (`DefaultBindings.map`, schema tastiera + mouse): WASD movimento,
frecce visuale, Spazio gambe, clic sinistro mano armata, E testa, Maiusc sinistro mano nuda,
clic destro profilo alto, Tab mappa, Esc pausa. I pulsanti del mouse sono i codici 256 + N.

Stato delle verifiche: la finestra si apre via **Wayland** (WSLg) e mostra il gioco; i tasti
del canale di comando funzionano anche con la finestra aperta. La tastiera e il mouse **veri
non sono stati provati**: di notte non c'era nessuno a premere tasti, e non c'è modo di
iniettarli (su X11, dove esisterebbe XTEST, a winit manca `libxkbcommon-x11.so` e la finestra
non si apre: in quel caso il runtime prosegue fuori schermo). Il gioco legge il mouse come
stato immediato `DIMOUSESTATE2` (20 byte) a ogni fotogramma, e all'avvio enumera gli oggetti
dei dispositivi con `EnumObjects`: per il mouse elenchiamo tre assi e otto pulsanti.

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

- **Ancora un falso inizio.** Scelto `NEW`, il caricamento si fermava su `0x005830F0`: una
  funzione vera, allineata e preceduta da riempimento, scartata perché un indirizzo "tondo"
  (`0x00583420`, una parola nei dati) era stato accettato prima e cadeva in mezzo a una sua
  istruzione. La scoperta ora lavora in tre passate, dalla più severa: prima le sole partenze
  precedute da riempimento o da un `ret`, poi quelle soltanto allineate, infine le voci di
  tabella. `ALTAIR_WHY=0xADDR altair discover …` spiega perché una partenza è accettata o no.

## Prossimi passi

1. Giocare davvero con `tools/gioca.sh`: tastiera e mouse veri non sono mai stati provati.
2. Uscita audio: capire perché PulseAudio di WSLg consuma a singhiozzo (sezione Audio).
3. Completare il tutorial dell'Animus e arrivare a Masyaf; controllare che il salvataggio
   automatico funzioni lungo il percorso.
4. Punti aperti: SEH, file INI veri, prima pagina protetta, suono 3D, controller.
