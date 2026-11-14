# scimitar-recomp: cosa ci dice su AC1

Note di lettura (2026-11-14) di `ref/scimitar-recomp` al commit `4f36655` (2026-11-10):
`docs/AC1_WINDOWS.md` e `docs/PLAN.md`. Riassunte con parole nostre; il codice è GPL-2.0 e non
si copia. Dove scrivono "stima", è una loro stima.

## La loro build e la nostra sono lo stesso codice

Confronto fatto qui con `tools/pe.py` e lettura delle directory PE. Tutti i valori che
riportano per la build Ubisoft Connect coincidono con la nostra build Steam:

| | Ubisoft Connect (loro) | Steam (nostra) |
|---|---|---|
| Timestamp PE | 2008-04-23 | 2008-04-23 19:45:11 UTC |
| Entry point | `0xF9F445` (`WinMainCRTStartup`) | `0xF9F445` |
| Sezioni `.text` … `.rsrc` | stessi indirizzi | stessi indirizzi |
| Template TLS | 673 byte, indice a `0x1A3561C` | uguale |
| Cookie `/GS` | `0x19CA838` | `0x19CA838` |
| Export | 178 (motore audio DARE) | 178 |
| Import | 261 da 17 DLL, tra cui `uplay32` | 260 da 17 DLL, senza `uplay32` (c'è `iphlpapi`) |
| Sezione in più | `.nidata` a `0x1BBD000` (4 KB) | nessuna |
| Dimensione | 24.671.912 byte | 24.666.760 byte (firma Authenticode di 4.744 byte in coda) |

**Conclusione.** Ubisoft Connect ha solo aggiunto una sezione `.nidata`, con una copia della
tabella di import più `uplay32.dll!Test`, che nessuno chiama. Il codice ha gli stessi indirizzi.
Gli indirizzi e i conteggi di scimitar-recomp valgono quindi anche per la nostra build.
Resta da fare una conferma byte per byte di `.text` (vedi `domande-aperte.md`), ma ormai è una
formalità.

Nota: noi contiamo 17 DLL e 260 funzioni. La loro lista ne nomina 17 includendo `uplay32`;
`iphlpapi` compare in entrambe. La differenza di una funzione è `uplay32!Test`.

## L'eseguibile, dai loro numeri

- **`.text` (19,4 MB)**: contiene un blocco di 3,5 MB di `int3` mai referenziato
  (`0x11B5239`–`0x1520860`), seguito dai funclet di unwind C++.
- **Istruzioni**: lo sweep lineare ne decodifica 9,85 milioni, di cui 5,44 milioni `int3`.
  Seguendo il flusso dalle funzioni trovate: 91.969 funzioni e 3,31 milioni di istruzioni.
  Con la scoperta finale: **98.470 funzioni, 3.484.935 istruzioni**. Ne traducono 98.453;
  le 17 restanti sono partenze deboli che non sono codice.
- **RTTI**: 4.376 type descriptor (3.407 nel namespace `scimitar`), 4.597 vtable trovate
  dai Complete Object Locator, 59.899 slot, 28.717 funzioni distinte.
- **Import per DLL**: KERNEL32 157, USER32 32, WS2_32 30 (per ordinale), binkw32 14,
  ADVAPI32 7, ole32 3, XINPUT1_3 3, SHELL32 2, DSOUND 2, d3dx9_36 2, WININET 2, e una
  ciascuna da d3d9, DINPUT8, EAX, OLEAUT32, WINMM, iphlpapi. CRT linkato staticamente.
- **Il motore audio DARE di Ubisoft** è linkato dentro ed esporta 178 funzioni
  (`DareObjectFactory`, `SND_fn_*`). Gira su DirectSound + EAX.

## Censimento delle istruzioni difficili (sweep lineare)

| Cosa | Quante | Nota |
|---|---|---|
| Accessi `fs:` (catena SEH, TEB) | 46.844 | da soli bloccavano 12.905 funzioni |
| `ret imm` (stdcall/thiscall) | 76.904 | il chiamato toglie gli argomenti dallo stack guest |
| Confronti x87 (`fcom*`, `fucom*`, `fnstsw`, `sahf`…) | ~7.000 | |
| Funzioni x87 (`fsqrt`, `fsin`, `fpatan`…) | ~60 | nella matematica del CRT |
| `lock xadd` / `lock cmpxchg` | 9.188 | `Interlocked*` in linea |
| `pushfd`/`popfd`, `bt*`, `rcl`/`rcr`, `loop*` | qualche centinaio | CRT, controlli CPUID |
| MMX/SSE2 su parole intere | ~300 | mixer di DARE |
| Jump table | 728 | anche a due livelli (tabella di byte prima) |

## Scoperta delle funzioni senza simboli

- **Partenze forti**: entry point, export, callback TLS, e ogni destinazione di `call`
  diretta nel codice raggiunto da queste.
- **Partenze deboli**: slot di vtable; parole nei dati che puntano nel codice e sembrano
  un inizio (MSVC allinea il 96% delle destinazioni a 16 byte dopo padding `int3`);
  immediati con indirizzi di codice (`push offset callback`); indirizzi dopo `ret` e
  thunk `jmp rel32` del linker.
- Una funzione è il codice raggiungibile dalla sua partenza senza passare per un'altra
  partenza (passarci è una tail call). Non ci sono estensioni fisse: il codice può essere
  condiviso, e una partenza debole sbagliata costa codice duplicato, non codice errato.
- Le partenze deboli si scartano, iterando, se cadono dentro un'istruzione di un'altra
  funzione, se sono destinazione di un salto condizionato altrove o se non decodificano.
  Solo le funzioni forti contano come prova contro le deboli.
- `call [IAT]` e `jmp [IAT]` sono chiamate di import. Le celle IAT vanno anche riempite
  con funzioni host, per il pattern MSVC `mov esi, [__imp_X]; call esi`.
- Jump table: forma a un livello, forma a due livelli, tabelle di `memcpy` indicizzate
  da 1 e da −3.

## Insidie trovate a runtime (da non ripetere)

1. **Codice sotto l'entry di una funzione.** Una funzione PE può saltare a codice prima del
   suo inizio. Il corpo emesso in ordine di indirizzo deve cominciare con un salto all'entry.
2. **Flag tra funzioni spezzate.** Se la scoperta fa iniziare una funzione dentro un'altra,
   il passaggio è una tail call e i flag vanno passati nel contesto.
3. **Jump table di MSVC.** Indici negativi in `memcpy`/`memmove`; il confronto può stare
   diverse istruzioni prima del salto; il limite di una tabella a due livelli si ricava
   dalla tabella di byte.
4. **Thread.** La finestra vive su un thread del motore mentre il thread principale aspetta.
   I frame tradotti richiedono stack host grandi: **256 MB** per i thread di gioco.
5. **Render target nuovi da azzerare.** Il gioco legge target che non ha disegnato del
   tutto (bordi della catena di bloom).
6. **Draw che campiona il proprio render target** (post-processing): serve una copia
   presa subito prima.
7. **Render target L8 letti come texture**: serve uno swizzle sulla vista, altrimenti i
   menu diventano rosa.
8. **Tempo di compilazione.** Una funzione x87 da 29.000 istruzioni ha tenuto l'ottimizzatore
   di clang per più di 15 minuti. Oltre 4.000 istruzioni x87 non ottimizzano.
   Generano in parti scelte per indirizzo; i file invariati non si ricompilano.

## Stato del loro port (2026-11-10)

- Arriva a schermata del titolo, profili, menu, laboratorio Abstergo e **tutorial
  nell'Animus**, con shadow map 4096x1024, depth pass, bloom e color grading.
  900–2.000 draw per frame, circa 50 fps a 2560x1440.
- Il frame gira su **un solo thread**; i quattro worker ricevono poco lavoro. Il costo
  principale è il codice tradotto (circa metà) e poi il runtime x87.
- Ottimizzazioni che hanno contato: upload delle costanti senza scansione per parametro,
  `com::as<T>` con tag di tipo al posto di `dynamic_cast`, pad letto in snapshot, percorsi
  veloci bit-esatti per x87 (2x).
- **Non verificati**: partita a mano, ricordi dopo il tutorial (Tempio di Salomone, Masyaf),
  salvataggio e caricamento, draw fixed-function/FVF, sRGB, audio udito, metodi D3D9
  mancanti (ognuno ferma l'esecuzione con il suo nome).

## Stime per una persona, fatte prima del lavoro

| Pezzo | Stima |
|---|---|
| Copertura difftest delle nuove istruzioni | 2–3 giorni |
| Compilare l'immagine intera | 1–2 giorni (C++ circa 1 GB, generato a parti) |
| Runtime Windows (TEB, TLS, stdcall, COM) | 3–5 giorni |
| KERNEL32, USER32 e DLL minori | 2–3 settimane |
| Direct3D 9 (circa 120 metodi del device, shader, D3DX constant table) | 6–10 settimane |
| DirectInput 8 + XInput | 1 settimana |
| DirectSound 8 sotto DARE | 2 settimane |
| Bink (14 funzioni, via FFmpeg) | 2 giorni |
| Winsock: fallire come non disponibile | 1 giorno |

## Dettagli di comportamento utili

- Il gioco trova i pad tramite DirectInput e passa un controller Xbox 360
  (vendor `0x045E`, product `0x028E`) al percorso XInput (`PadXenon`).
- Il gioco vede 4 processori di default nel loro port (`SCIMITAR_WIN32_CPUS`).
- Salvataggi: file normali tramite `SHGetSpecialFolderPathA` e I/O del CRT. Il profilo è un `.hdr`.
- Winsock: basta che fallisca come "non disponibile". Uplay: niente.
- **Nomi da AC2 Mac**: 4.978 funzioni con nome per corrispondenza, tra cui `MainCommon::Initialize`,
  `EngineLoop::Enter`, `Engine::RegisterClasses`. Altre 19.664 ricevono classe e slot dalla
  RTTI di AC1. Circa 85.000 restano senza nome.

## Archivi `.forge` di AC1 (da `AC1_WINDOWS.md` e `PC_DATA.md`)

- Il contenitore ha lo stesso header di AC2: magic `scimitar`, **versione 25**. Indice e
  layout a blocchi sono come quelli di AC2.
- Il record FILEDATA prima dei dati di ogni file è **4 byte più corto** che in AC2: il magic
  del blocco compresso sta a `+0x9B8`, in AC2 a `+0x9BC`.
- I blocchi di AC1 sono **LZO1X_999 (tipo 1)**. Per confronto: AC2 Mac usa LZO1X_1 (tipo 0),
  AC2 PC LZO2A (tipo 2).
- Le classi negli archivi si identificano con il **CRC-32 del nome della classe**
  (es. `ShaderPermutation` = `0x9467F2BB`).
- I record FastLoad sono i campi in ordine, **senza tag né versioni**: il layout di una classe
  cambia da gioco a gioco, quindi non si possono usare i dati di un gioco con l'eseguibile
  di un altro. Per lo stesso motivo non si riesce a far girare AC1 sull'eseguibile Mac di AC2.
- Il primo file del primo archivio è `GlobalMetaFile`, seguito da `Game Bootstrap Settings` e
  `Game Fix`.
- Gli shader negli archivi PC sono **bytecode D3D9** con prefisso di lunghezza, dentro i record
  `ShaderPermutation` / `LightingShaderPermutation`. Il motore PC costruisce la mappa delle
  costanti al caricamento con `D3DXGetShaderConstantTable`: per questo è una delle due funzioni
  d3dx9 importate.

## Grafica: lezioni da AC2 PC data e Revelations (`PC_DATA.md`, `ACR_WINDOWS.md`)

Valgono per il nostro D3D9 su wgpu, perché il layer D3D9 è condiviso tra AC1, ACR e PoP.

**Traduzione degli shader.** Loro passano da D3D9 ad ARB e poi a MSL. ARB non ha controllo di
flusso, quindi devono appiattire `if`/`else` (eseguendo entrambi i rami) e srotolare i `rep`.
L'appiattimento ha causato NaN: un infinito nel ramo non preso, moltiplicato per una maschera 0,
dà NaN, e metà di Firenze spariva. **Noi traduciamo direttamente D3D9 → WGSL**, che ha controllo
di flusso vero, e il problema non si pone. Altre cose che valgono anche per noi:

- matrici column-major nella constant table, da leggere trasposte;
- registri bool: vanno dati come costanti float separate (i bool usati in ACR sono
  `g_VolumeFogEnabled`, `g_UseDepthTextureShadows`, `g_DeconstructionEnabled`, `g_HasSun`, …);
- `texldl` con LOD esplicito, `vFace`, offset relativi grandi;
- le costanti bool vanno messe dopo i registri float (le palette di ossa arrivano a c255).

**Bug Direct3D 9 trovati in Revelations e condivisi con AC1:**

1. **Le uscite colore di `vs_3_0` non vanno saturate.** Il gioco scrive profondità lineare in
   un render target R32F attraverso un'uscita `COLOR`. Il clamp a 0..1 vale solo sotto SM3.
2. **`vPos` è la coordinata intera del pixel.** Nelle API moderne la posizione è il centro
   del pixel (k + 0.5): bisogna sottrarre 0,5.
3. **Mezzo pixel.** Il pixel k di D3D9 è il k + 0,5 delle API moderne: le posizioni vanno
   spostate di mezzo pixel *a destra e in basso* in clip space. Usano i 63/64 di mezzo pixel,
   come Wine. Con l'errore, la catena di bloom (da 320x180 a 20x11) produceva un bordo luminoso.
4. **Depth buffer più grande del render target**: in D3D9 è lecito; va scartato solo se è più
   piccolo.
5. **State block** registrati: il gioco ne usa due per resettare gli stati, migliaia di volte
   al minuto.
6. **Rinominare buffer e texture** riscritti dopo che un draw registrato li ha usati.
7. **Device a schermo intero**: `CreateDevice` o `Reset` a schermo intero ridimensionano la
   finestra al back buffer, e il gioco riceve `WM_SIZE`. Senza questo, ACR restava nero.
8. Altro risolto per AC1: scalatura di `StretchRect`, render target NULL, `Reset` che conserva
   il back buffer, il device che tiene riferimenti a ciò che ha legato.

**Runtime Win32:**

- `QueryPerformanceCounter` a **10 MHz** dall'avvio del processo: i nanosecondi dall'avvio del
  sistema mandavano in overflow il calcolo `ticks * 1000000 / frequency` del gioco.
- Con un pad collegato il gioco chiede effetti di vibrazione a DirectInput: `CreateEffect`
  deve rispondere "non supportato" invece di interrompere.
- **Numero di processori.** In ACR, dichiarando 2 o 4 CPU lo sfondo del menu lampeggia tra due
  proiezioni: è una race nel percorso multiprocessore del gioco, che il loro timing rende
  visibile. ACR ora dichiara 1 CPU; AC1 resta a 4 (`SCIMITAR_WIN32_CPUS`). Da tenere d'occhio.
- Il gioco crea e gestisce la finestra su un thread del motore, non sul thread principale.

## Scoperta delle funzioni: perché AC1 è più difficile (`ACR_WINDOWS.md`)

- ACR ha rilocazioni (862.811) e una tabella SafeSEH di 34.150 handler: le rilocazioni dicono con
  certezza quali parole sono indirizzi. **AC1 non ha rilocazioni**: ogni parola nei dati che cade
  nel range del codice è una partenza candidata, ed è da lì che vengono le false "istruzioni"
  `in`/`out`/`les`.
- Con le rilocazioni ACR passa da 188.802 a 274.540 funzioni. Senza, mancavano funzioni raggiunte
  solo tramite vtable; per esempio il CRT salva l'indirizzo di un wrapper di `TlsAlloc` che segue
  un `ret`, senza padding.
- La scoperta "a sequenze di indirizzi di codice nei dati" usata per AC1 perde le etichette del
  dispatch x87 del CRT, salvate non allineate in `.data`.
- Fonte di nomi per AC1 dal loro confronto: **4.323 nomi RTTI** con una scansione di stringhe,
  2.582 dei quali `scimitar`. Il nostro `pe.py rtti` ne trova 4.043: va capito il perché
  (regex diversa? nomi annidati?) quando scriveremo il parser vero.

## Struttura del loro layer Windows (`platform/README.md`, `platform/win32/`)

Utile come mappa dei moduli del nostro runtime, non come codice:

| File | Cosa copre |
|---|---|
| `core.cpp` | handle, attese, TEB e TLS statica, IAT |
| `kernel32.cpp`, `kernel32_sync.cpp`, `kernel32_file.cpp` | KERNEL32: base, sincronizzazione, file |
| `user32.mm`, `win32_window.h` | finestra e messaggi |
| `system.cpp`, `registry.cpp` | DLL piccole, registro emulato |
| `com.cpp/h` | oggetti COM in memoria guest |
| `d3d9.mm`, `d3d9_metal.mm`, `d3d9_internal.h` | API D3D9 e renderer |
| `dinput.cpp`, `input.cpp`, `xinput.mm` | input |
| `dsound.cpp` | DirectSound |
| `bink_win.cpp` | Bink (wrapper stdcall su FFmpeg) |
| `games/ac1/main.cpp`, `game.cpp` | avvio e codice specifico di AC1 |

Le funzioni host chiamabili dal guest hanno un indirizzo guest proprio (`kHostFnBase`); dal
lato host si richiama il guest con `call_guest` / `call_virtual`.

## Traduttore (`recomp/README.md`)

- Ogni funzione diventa `void f_XXXXXXXX(Context& c)`: registri nel contesto, memoria guest
  tramite base più offset, salti interni come `goto`, chiamate che spingono il vero indirizzo
  di ritorno sullo stack guest, tail call come `f_target(c); return;`.
- Le chiamate indirette passano da un registro delle funzioni tradotte e host. Le jump table
  diventano `switch` sulle etichette della tabella. I flag sono booleani locali, tenuti solo
  dove un'istruzione successiva li legge.
- x87 a 80 bit esatti: **450.000 risultati confrontati con Unicorn senza differenze**, salvo due
  punti in cui QEMU differisce dall'hardware (overflow di `fisttp`, conversioni di NaN
  segnalanti). Si segue l'hardware.
- SSE con la semantica x86 su qualunque host: NaN di default negativo, conversioni float→int
  che danno `0x80000000`, regola degli operandi di min/max. `rcp`/`rsqrt` sono esatti come in
  Unicorn, mentre le CPU reali danno approssimazioni a 12 bit: nei test serve tolleranza o emulazione.
- `rdtsc` come contatore a 1 GHz dal tempo host; `cpuid` come CPU fissa di classe SSE3.
- Il loro difftest su AC2 ha confrontato 1.500 funzioni scelte per coprire ogni tipo di
  istruzione, con 8 casi ciascuna: 8.889 casi identici a Unicorn.
- Immagine intera di AC2: 947 MB di C++ in 64 file, compilati in 125 s su 10 core a `-O1`.

## Backend grafico (`METAL.md`, solo i punti che valgono per wgpu)

- La superficie grafica usata dal gioco è stretta; si implementa solo quella. Il censimento di
  un frame (chiamate per tipo) è il primo passo, ed è da rifare per D3D9 su AC1.
- **Precompilazione e cache delle pipeline**: senza, ogni nuova coppia di shader costa circa
  30 ms la prima volta.
- Depth: un formato diverso dal D24 originale cambia la scala del polygon offset.
- Occlusion query: attenzione alle attese se la GPU è più di un frame indietro.

## Migliorie del loro port (`README.md`)

Un elenco utile per i nostri "nice to have", perché mostra cosa si ottiene con lo strato di traduzione:
qualsiasi formato di schermo con l'HUD ai bordi, distanza di visibilità fino a 6x, ombre
4096x4096, filtro anisotropico 16x, risoluzioni native, launcher con impostazioni,
cache degli shader, alcune routine matematiche calde sostituite da codice nativo equivalente.

Loro stessi scrivono che ricostruire sorgenti leggibili del motore richiederebbe anni, e che
non sarebbero pubblicabili perché derivati dal codice Ubisoft. È coerente con la nostra scelta
della decompilazione mirata.
