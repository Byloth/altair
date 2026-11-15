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

### Il tutorial, passo per passo (2026-11-15 mattina)

Con i comandi simulati di tastiera e mouse (`tools/ctl.sh`), dopo `tools/avvia.sh --animus`:

| Passo | Cosa chiede il gioco | Comando | Esito |
|---|---|---|---|
| Soggettiva | "Premi Testa da fermo" | `key 18 60` (E) | accettato, cambia scena |
| Scostare | "Tieni premuto Mano nuda mentre cammini", raggiungere l'indicatore | `key 42 300` + `key 17 300` (Maiusc + W) | Altaïr attraversa la fila di monaci e raggiunge la luce |
| Alto/basso profilo | "Premi senza rilasciare" Alto profilo | `button 1 300` (tasto destro) | accettato, compare un bersaglio |

| Agguantare e scagliare | "Premi senza rilasciare (Mouse destro) e premi (Maiusc sinistro)", scagliare l'uomo sull'indicatore | — | **non superato**: vedi sotto |

`tools/avvia.sh --tutorial` esegue da solo i primi tre passi (durate in tempo reale, suffisso
`ms` nei comandi `key` e `button`) e lascia il gioco davanti al quarto.

Il quarto passo chiede di avvicinarsi a un uomo, afferrarlo e orientare il lancio: pilotando
con una schermata ogni qualche secondo ho perso il bersaglio dalla visuale al secondo
tentativo. Non è un difetto del gioco tradotto (i comandi arrivano), è il limite di questo modo
di giocare: da qui serve una persona. Il salvataggio non cambia durante il tutorial (ultima
scrittura del `.sav`: la nuova partita), quindi il tutorial va fatto in una sola sessione.

La barra di sincronia sale di una tacca a ogni passo (da 1 a 4). Quindi tastiera, pulsanti e
movimento del mouse arrivano tutti al gioco e il tutorial è percorribile. Mi sono fermato al
bersaglio: i passi successivi (combattimento) è meglio giocarli che scriverli.

Difetti visti:

- **La scena con la folla è lenta**: 3.800 draw, 9 fotogrammi al secondo (51 ms nel renderer,
  di cui 35 di invio; il resto del fotogramma è il gioco stesso).
- ~~Il nome del tasto nei messaggi è vuoto~~ **Risolto.** Il gioco crea `IDirectInput8W`
  (interfaccia Unicode, GUID `BF798031-…`), quindi `EnumObjects` consegna
  `DIDEVICEOBJECTINSTANCEW` con il nome in UTF-16; noi lo scrivevamo in ASCII. La callback del
  gioco (`0x009091C0`) copia il nome dei pulsanti (`GUID_Button`, indice `dwOfs - 12`) e dei
  tasti (`GUID_Key`, indice `dwOfs` = codice DIK) in una tabella da 0x208 byte per voce. Ora
  elenchiamo anche i tasti della tastiera, con nomi italiani: il messaggio dice "Premi senza
  rilasciare (Mouse destro)".

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

## File INI

`system.rs`. Le funzioni `GetPrivateProfile*` e `WritePrivateProfileStringA` leggono e scrivono
file veri nel disco scrivibile (`run/c/…`); la cartella del gioco resta in sola lettura. Alla
prima corsa il gioco crea, in `C:\Users\Player\AppData\Roaming\Ubisoft\Assassin's Creed\`:

- `Assassin.ini`: `[Player] LastUsedProfile=<numero del profilo>`;
- `DARE.INI`: opzioni del motore audio (`DS3D_CACHE_SIZE`, `NB_AUDIBLE_VOICES=48`, …).

Con `LastUsedProfile` il percorso dei menu all'avvio resta quello di prima (verificato:
`tools/avvia.sh --partita` arriva ancora al laboratorio).

## Puntatori nulli e accessi non validi

I primi 64 KB dello spazio guest sono protetti, come in Windows (`ALTAIR_NULL=0` toglie la
protezione), e un gestore di `SIGSEGV` stampa l'indirizzo guest, il thread e le ultime
destinazioni di chiamate indirette, poi esce con codice 3. Prima un accesso sbagliato dava un
core dump muto, e una lettura da un puntatore nullo passava inosservata. Percorso completo
fino al corridoio dell'Animus con la protezione attiva: nessun accesso non valido.

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

### Il suono che gracchia: il thread audio del gioco arrivava tardi (2026-11-15)

Segnalato dall'utente alla prima prova su Windows: nel menu e in partita, sopra l'audio giusto,
un secondo audio "rotto"; i filmati invece puliti. È la prima volta che qualcuno ascolta.

Come il gioco usa i buffer (visto con `ALTAIR_AUDIO_LOG=1`, che stampa Play, Stop e ogni
`Lock` con cursore, istante e thread): ogni voce è un buffer da 19.200 byte, mono a 16 bit,
suonato in ciclo a 24-36 kHz (`SetFrequency`), diviso in cinque segmenti da 3.840 byte, circa
60 ms. Un thread dedicato (quello che chiama `SetThreadPriority`) interroga la posizione e
riempie **il segmento successivo a quello in cui sta il cursore**: ha quindi al più 60 ms di
margine. Se arriva tardi, il mixer suona un segmento vecchio di un giro: suono giusto con
sopra pezzi sbagliati. I filmati non ne soffrono perché il loro audio è già tutto decodificato.

Perché arrivava tardi: ogni sua chiamata (`GetCurrentPosition`, `Lock`, `Unlock`, `GetStatus`,
`QueryPerformanceCounter`) prendeva il lock globale del runtime, conteso dal thread che
disegna. Misura su un minuto, dal menu al laboratorio: **123 riempimenti arrivati con più di
100 ms di distanza** dal precedente.

Correzioni:

- i metodi dei buffer e l'orologio non prendono più il lock del runtime (shim `F`);
- il mixer non recupera più il tempo perso avanzando i cursori a scatti;
- su Windows, `timeBeginPeriod(1)` all'avvio: i timer di sistema passano da 15,6 ms a 1 ms.

Dopo: 70 ritardi nello stesso percorso, concentrati all'avvio e durante i caricamenti (lì il
thread audio aspetta i lock del gioco stesso); a regime, nel menu e nel laboratorio, nessuno.
Il mixer ora li conta: stampa "N ms di flussi suonati da zone non riscritte in tempo" ogni
5 secondi in cui capita. **Da riascoltare su Windows.**

### Il suono che gracchia, la causa vera: `floor(11,5) = 12` (2026-11-15)

La correzione qui sopra era reale ma non era quella: su Windows il registro non mostrava quasi
più ritardi e il suono gracchiava lo stesso. L'utente ha dato l'indizio giusto: gracchiano
**solo la musica del menu e le voci**, non l'ambiente, non i caricamenti, non i filmati.

Musica e voci sono **Ogg Vorbis**, decodificate da una libvorbis compilata dentro il gioco;
gli altri suoni no. La catena, risalita all'indietro dal buffer DirectSound (tutto verificato
qui, con sonde temporanee nel C generato e con `ALTAIR_AUDIO_TRAP`):

| Stadio | Indirizzo | Esito |
|---|---|---|
| Buffer di miscelazione DirectSound (stereo 48 kHz, 37.888 byte) | oggetto `0x10013750` | fotogrammi alterni di due segnali diversi, picchi |
| Conversione float → int16 di DARE (`cvttps2pi`…) | `0x8383A0` | tradotta bene; riceve float già guasti |
| `vorbis_synthesis_blockin`, somma con sovrapposizione | `0x83FF10` | tradotta bene; sorgente già guasta |
| `vorbis_synthesis` | `0x841590` | |
| `mapping0_inverse` (tabella `0x1927740`) | `0x851A60` | sano dopo residui, accoppiamento stereo e curva; **guasto dopo la MDCT** |
| `mdct_backward` | `0x84ECB0` | identica all'originale se provata da sola |
| `mdct_bitreverse` | `0x84EB40` | legge fuori dal buffer |
| `mdct_init` | intorno a `0x84F2D8` | **`log2n = 12` per n = 2048** (giusto: 11); per n = 256 dà 8, giusto |

`mdct_init` calcola `log2n = (int)floor(log(n)/log(2) + 0,5)`. Per n = 2048 è `floor(11,5)`, e
il `floor` del CRT tradotto restituiva 12: `frndint` ignorava la modalità di arrotondamento
(dettagli in `traduttore.md`, "Cosa ha trovato il test", punto 7). Per n = 256, `floor(8,5)`
dava 8 solo perché l'arrotondamento al più vicino manda i mezzi al pari. Con `log2n` sbagliato
la tabella di inversione dei bit contiene indici fino a 2046 invece che fino a 1022: la MDCT
dei blocchi lunghi legge memoria oltre il proprio buffer e la mescola al segnale. Per questo
sotto si sentiva il suono giusto, e per questo l'originale in Unicorn con le stesse tabelle
dava lo stesso risultato del tradotto (stessa memoria vicina) ma non quello del gioco.

Due cose in più, emerse per strada:

- Il gioco finiva nel ramo x87 di `floor` solo perché il CRT credeva che la CPU non avesse
  SSE2 (bit ID di EFLAGS perso, punto 8 dello stesso elenco). Su un PC vero prende il ramo
  SSE2. Corretto anche questo: ora la causa è tolta due volte.
- DARE sceglie tra due implementazioni delle farfalle della MDCT (`0x84E960` e `0x84EA30`)
  secondo la variabile `0x1A19ECC`, impostata all'avvio da `0x81A250` (rilevamento della CPU;
  nel gioco vale 1). Tutte e due provate con dati veri: identiche all'originale.

Verifica dopo la correzione, sul buffer di miscelazione nel menu principale (70 s, campioni
sopra 300 in valore assoluto): la differenza media tra fotogrammi consecutivi è 27,7 e quella
a distanza due 53,7, come in un segnale continuo; prima era il contrario (la distanza due era
molto più piccola: due segnali intercalati). Nessun campione saturato, massimo 7.076.
**Da riascoltare su Windows.**

Lezione: **una funzione identica all'originale in isolamento può essere guasta nel gioco se le
sue tabelle sono nate male.** Il confronto va fatto con lo stato del gioco, non con ingressi
inventati; qui è servito salvare ingresso e uscita di 600 chiamate vere e rieseguirle
sull'originale. Un dato salvato per la prova (`private/audio/mdct-2048.raw`) resta fuori da git.

Strumenti nati dall'indagine, tutti spenti di norma:

- `ALTAIR_AUDIO_LOG=1` ora salva anche ciò che il gioco scrive nei buffer in flusso, in
  `run/ctl/flusso-<oggetto>.raw`, e stampa formato e flag di ogni `CreateSoundBuffer`;
- `ALTAIR_AUDIO_TRAP=N` (Linux): all'N-esimo `Lock` del buffer di miscelazione la zona diventa
  di sola lettura; con `ALTAIR_TRAP_CONTINUE=1` il gestore degli accessi non validi nomina la
  funzione tradotta che ci scrive (`dladdr`) e lascia proseguire.

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

Non c'è la libreria di RAD. **Dal 2026-11-15 i filmati li decodifica un decodificatore
nostro**, il crate `crates/bink` (vedi `analisi/bink.md`): video identico al riferimento,
audio in stereo (fronte + centro + retro) passato al mixer man mano, con venti fotogrammi di
anticipo sul video. Nessun programma esterno. `ALTAIR_BINK=ffmpeg` torna alla strada di prima
(un processo `ffmpeg` letto da una pipe), che resta come confronto; `ALTAIR_VIDEO=0` fa
fallire `BinkOpen` e il gioco salta il filmato. Invio salta il filmato introduttivo.

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
- **F8** (non arriva al gioco) mostra o nasconde il pannello di diagnostica in sovrimpressione:
  vedi "Pannello di diagnostica" più sotto;
- **F10** cattura o libera il puntatore (non arriva al gioco): catturato, il mouse muove la
  visuale senza fermarsi ai bordi; libero, serve per i menu.

Senza la variabile tutto resta fuori schermo, come prima. `tools/gioca.sh` la imposta.

**In finestra il ritmo è legato allo schermo.** La superficie offre una sola modalità di
presentazione, `Fifo` (stampato all'avvio: "modalità di presentazione offerte: [Fifo]"): con
il backend OpenGL di wgpu su Wayland `Present` aspetta sempre il refresh. Quindi non più di
60 fotogrammi al secondo, e una scena che fuori schermo ne fa 45 in finestra scende a 30
(perde un refresh su due). Presentare da un altro thread non aiuterebbe: lo scambio dei buffer
tiene il lock del contesto OpenGL, che serve anche al thread del gioco. Limite di WSLg con
OpenGL, non nostro; con Vulkan o D3D12 nativi le modalità senza attesa esistono.

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

### Pannello di diagnostica (F8, 2026-11-15)

Richiesta dell'utente: niente numeri nel titolo della finestra, ma un indicatore disegnato a
video, curato e nello stile dell'Animus, "che ricordi che tutto il gioco si svolge dentro a
una macchina". Prima si è cercato un indicatore originale da riaccendere: non c'è
(`funzioni-nascoste.md`).

`overlay.rs` costruisce il pannello come elenco di triangoli colorati, in pixel della
finestra; `gfx.rs` lo disegna sulla superficie dopo aver copiato il back buffer, con una
pipeline a parte (alfa, nessuna texture) e un buffer di vertici nuovo per fotogramma. Il gioco
non lo vede: il suo back buffer resta intatto. Contenuto: un'elica di rombi che ruota,
"ANIMUS / SINCRONIA", i fotogrammi al secondo in grande, millisecondi per fotogramma, draw per
fotogramma, e uno storico a barrette. Sotto i 30 fotogrammi al secondo numeri e accenti
passano al rosso della desincronizzazione. I valori si aggiornano due volte al secondo.

Lo storico: 56 barrette, **una ogni quarto di secondo** (14 secondi in tutto), le più vecchie a
sinistra e più tenui. L'altezza è il ritmo medio di quell'intervallo, su una scala a gradini
fissi (30, 60, 120, 240, 480…) scelta sul massimo visibile: cambia solo quando il ritmo cambia
davvero. Una barretta è rossa se l'intervallo è stato sotto i 30 fotogrammi al secondo o ha
contenuto un fotogramma più lungo di 50 ms; le soglie sono assolute, quindi non cambia colore
dopo. La prima versione aveva una barretta per fotogramma, con altezza e rosso relativi alla
media del momento: scorreva via in mezzo secondo, si ridimensionava a ogni ricalcolo e le
barrette cambiavano colore (segnalato dall'utente, illeggibile).

**Niente asset del gioco:** caratteri 5×7, elica e cornice sono descritti in codice. Il logo
Abstergo non c'è, apposta: è un marchio di Ubisoft e la regola 1 lo tiene fuori dal repository.

Si accende con F8, con il comando `fps` del canale di comando o con `ALTAIR_FPS=1`. Con il
pannello acceso, `shot` salva ciò che si vede in finestra (back buffer più pannello, composti
su una copia). Verificato su WSL nel laboratorio (`run/scatti/pannello-1.png`); su Windows da
provare.

**F8 e il gioco.** Il tasto è intercettato in `window.rs` e non raggiunge mai DirectInput, come
F10. Per sapere se il gioco perde qualcosa: iniettato il codice DIK 66 (F8) dal canale di
comando nel laboratorio, nessun effetto visibile; nel codice tradotto nessun test del tipo
`[stato+42h] & 80h`. Non è una prova di assenza: i tasti sono rimappabili dalle opzioni.

### Filmati iniziali saltabili (2026-11-15)

L'originale lascia saltare solo `DemoIntro.bik`: logo Ubisoft, "presenta" e avvertenze vanno
guardati per intero. Il permesso sta in una tabella nei dati del gioco, quella dello
`StartupVideoManager`: voci da 36 byte a partire da `0x18C2C08`, lette dal codice intorno a
`0x41D45D-0x41D6A6`. Verificato qui sul file e nel codice tradotto:

| Offset | Significato | LoadingScreen | UBI_LOGO | Ubisoft_Presents | Warning_Disclaimer | DemoIntro |
|---|---|---|---|---|---|---|
| +0 | puntatore al nome | | | | | |
| +4 | **si può saltare** (byte) | 0 | 0 | 0 | 0 | 1 |
| +8 | dissolvenza in entrata, ms | 0 | 1000 | 1000 | 1000 | 1000 |
| +12 | durata, ms (−1: tutto il filmato) | −1 | −1 | 4000 | 8000 | −1 |
| +16 | dissolvenza in uscita, ms | 1000 | 1000 | 1000 | 1000 | 1000 |
| +20 | colore della dissolvenza | bianco | bianco | bianco | bianco | bianco |
| +24 | altri flag (le due schermate fisse hanno `01 01`) | | | | | |

Il significato di +8, +16 e +24 è dedotto dai valori (ipotesi); quello di +4 è provato dal
risultato. All'avvio il runtime scrive 1 nel byte +4 delle voci 1, 2 e 3 (`main.rs`), e da lì
fa tutto il gioco, come per il filmato introduttivo: alla pressione parte la dissolvenza sul
bianco e il filmato si chiude. Misura (WSL, Invio 1,5 s dopo l'apertura): ognuno dei tre si
chiude 1,2-1,3 s dopo il tasto; il logo al fotogramma 82 di 257, senza accelerare.
`ALTAIR_INTRO_SKIP=0` lascia la tabella com'è.

Primo tentativo, scartato lo stesso giorno: far rispondere a `BinkWait` "non aspettare" dopo
un tasto. Il logo scorreva accelerato e le schermate fisse, che hanno una durata in
millisecondi e non in fotogrammi, non si accorciavano abbastanza (segnalato dall'utente).
Lezione: prima di aggirare il gioco dall'esterno, cercare se ha già la funzione e manca solo
il permesso.

## Windows nativo (2026-11-15)

Lo stesso runtime gira su Windows, dove wgpu usa Vulkan o Direct3D 12 senza passaggi
intermedi (in WSL ogni draw attraversa OpenGL → Mesa → D3D12). **Compila e si collega; non è
ancora stato eseguito**: da WSL non posso lanciare programmi di Windows.

Ciò che cambia tra i due sistemi sta in `host.rs`:

| | Linux | Windows |
|---|---|---|
| Libreria con il codice tradotto | `dlopen` di un `.so` | `LoadLibraryW` di una DLL |
| Memoria guest (in `runtime/altair.c`) | `mmap` pigro di 4 GB | `VirtualAlloc` con 4 GB impegnati subito (vedi sotto) |
| Pagina zero e accessi non validi | `mprotect` + `SIGSEGV` | `VirtualProtect` + gestore vettoriale delle eccezioni |
| Uscita dal codice guest | `setjmp`/`longjmp` | `__builtin_setjmp`/`__builtin_longjmp` |
| Uscita audio | PulseAudio (`libpulse-simple`) | `waveOut` (`winmm`) |
| Profilatore a campionamento | segnali | assente |

Scelte da ricordare:

- **4 GB impegnati.** Su Linux le pagine nascono quando si toccano. Su Windows una zona solo
  riservata farebbe fallire le funzioni di sistema che ci scrivono (una lettura da file in un
  buffer del gioco), quindi la si impegna tutta: non occupa RAM finché non serve, ma conta nel
  limite di RAM più file di paging.
- **Niente `longjmp` di sistema.** Su Windows a 64 bit srotolerebbe lo stack con il meccanismo
  delle eccezioni, attraversando i frame in Rust; le funzioni intrinseche del compilatore
  ripristinano solo i registri.
- **Esportazioni.** Una DLL non può esportare più di 65.535 simboli e le funzioni tradotte
  sono 117.000: si esportano solo le cinque che il runtime cerca (`ALTAIR_API`).
- Il gestore delle eccezioni si occupa solo degli accessi alla memoria guest; gli altri li
  lascia a chi di dovere (un driver può gestire i suoi).
- Scelta dell'adattatore: `ALTAIR_BACKEND=vulkan|dx12|gl`; senza, Vulkan, poi Direct3D 12.

```sh
tools/build-win.sh                                             # da WSL
```
```powershell
powershell -ExecutionPolicy Bypass -File tools\gioca.ps1      # da Windows, nella cartella del progetto
```

`gioca.ps1` apre la finestra e scrive il registro in `run\win\log.txt`, leggibile anche da
WSL: è il modo in cui si diagnostica ciò che su Windows non va. Come compilare: `ambiente.md`.

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
