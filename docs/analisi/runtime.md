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
| `system.rs` | COM, registro, file INI, data e ora, rete, audio (assenti) |
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
- **Eccezioni**: nessun SEH. `RaiseException` gestisce solo l'eccezione con cui MSVC dà un nome
  ai thread; ogni altra ferma tutto.

## Dove arriva oggi

Con 170 shim (108 in `win32.rs`, 31 in `system.rs`, 31 in `user32.rs`), senza alcuna modifica
al codice tradotto:

1. **L'avvio del CRT** completo: heap, TLS, ambiente, locale, riga di comando.
2. **Tutti gli inizializzatori statici** del motore.
3. **`WinMain`**: il gioco chiede Game Explorer di Vista (`CoCreateInstance`, classe non
   registrata), installa l'hook della tastiera, legge le cartelle speciali.
4. **Crea i suoi thread**: `Loading`, `TaskThread1` e altri, 8 in tutto, che girano davvero.
5. **Inizializza l'audio**: enumera i dispositivi, prova `EAXDirectSoundCreate8` e
   `DirectSoundCreate8`.

Circa 1.000 chiamate a import nel thread principale. Le più frequenti, in una corsa tracciata:
`EnterCriticalSection`/`LeaveCriticalSection` (14.500 ciascuna), `TlsGetValue` (6.200),
`HeapSize` (485).

**Si ferma qui:** senza un dispositivo audio il gioco mostra il suo messaggio
"Cannot initialize Sound Driver" e subito dopo chiama un metodo virtuale su un oggetto nullo
(`0x007F3DBB`: `call edx` con l'oggetto restituito da `0x007EFFE0`). Sul Windows vero sarebbe
un crash: il gioco originale non parte senza scheda audio. Serve quindi un DirectSound che
funzioni, anche muto. **Il gate della fase (aprire i `.forge`) non è ancora raggiunto**: fino
a qui il gioco non ha aperto nessun file.

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

## Prossimi passi

1. **Oggetti COM**: vtable in memoria guest i cui slot sono trampolini verso funzioni host,
   con un arresto parlante per ogni metodo non implementato.
2. **DirectSound muto** sopra quel meccanismo (`IDirectSound8`, buffer, listener 3D).
3. Proseguire fino all'apertura dei `.forge`; poi DirectInput e Direct3D 9 (Fase 3).
