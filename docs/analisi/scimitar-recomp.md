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

## Altri documenti in `ref/scimitar-recomp/docs/` da leggere

- `PC_DATA.md`: formato dei `.forge` per PC (serve alla 0.10).
- `ANVIL_FAMILY.md`: confronto tra i giochi del motore.
- `ACR_WINDOWS.md`: correzioni Direct3D 9 del 2026-11-06, condivise con AC1.
- `METAL.md`: progetto del backend grafico. Utile per il nostro su wgpu.
- `recomp/README.md`, `platform/README.md`: struttura del traduttore e del layer.
