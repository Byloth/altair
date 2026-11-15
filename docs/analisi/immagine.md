# L'immagine intera tradotta

Misurato il 2026-11-15 su questa macchina (28 core, 31 GB), con clang 18.1.3 e gcc 13.3.0.
È il gate della Fase 1: tutto `.text` tradotto in C che compila, verificato a campione.

```sh
source ~/.bash_exports && cargo build --release
target/release/altair image "$AC1/AssassinsCreed_Dx9.exe" --out build/gen/full    # dalla radice del progetto
make -C build/gen/full -j28                        # clang -O1 → build/libaltair_ac1-clang.so
make -C build/gen/full -j28 CC=gcc                 # → build/libaltair_ac1-gcc.so
make -C build/gen/full -j28 OPT=-O2 TAG=clang-O2   # → build/libaltair_ac1-clang-O2.so
.venv/bin/python tools/difftest.py "$AC1/AssassinsCreed_Dx9.exe" --auto 20000 --lib build/libaltair_ac1-clang.so
```

## Come sono organizzati i file

Tutto sta in `build/gen/full/`, fuori da git.

- **Un file C per finestra di 64 KB di indirizzi**: `part_00401000.c` contiene le funzioni che
  iniziano tra `0x401000` e `0x410FFF`. Sono 241 file; le finestre senza funzioni (il blocco di
  `int3` da 3,5 MB) non producono file.
- Ogni file dichiara in testa solo le funzioni che definisce o chiama: non c'è un header globale.
- `table.c`: la tabella indirizzo → funzione per le chiamate indirette, ordinata.
- `Makefile`: un oggetto per file, più il runtime, e il link in una libreria condivisa. Le
  variabili `CC`, `OPT` e `TAG` scelgono compilatore, ottimizzazione e nome dell'output.

Perché per indirizzo: una modifica tocca solo il suo file. Dividendo "ogni N funzioni", una
funzione scoperta in più farebbe slittare tutti i file successivi. Inoltre il generatore non
riscrive un file identico a quello esistente, quindi `make` ricompila solo ciò che è cambiato:
una seconda generazione senza modifiche riscrive 0 file.

Le funzioni con più di 5.000 istruzioni sono marcate `ALTAIR_BIG` e il compilatore non le
ottimizza. Oggi è una sola: `0x00A33AF0`, con 25.075 istruzioni (scimitar-recomp ne segnala una
da 29.000 che tiene clang per oltre 15 minuti). La soglia non è stata tarata: con 5.000 il
tempo di compilazione è già buono.

## Numeri

| | |
|---|---|
| Funzioni | 116.077 |
| Istruzioni (contate per funzione) | 3.786.982 |
| Generazione | 3,1 secondi |
| C generato | 357 MB in 242 file; il più grande 6,6 MB |

| Compilatore | Tempo reale | Tempo CPU | Picco di memoria per processo | Oggetti | Libreria |
|---|---|---|---|---|---|
| clang `-O1` | 49 s | 956 s | 0,4 GB | 81 MB | 75 MB |
| clang `-O2` | 55 s | 1.109 s | 0,4 GB | — | 71 MB |
| gcc `-O1` | 50 s | 986 s | 0,7 GB | 83 MB | 78 MB |

Per confronto, scimitar-recomp stima circa 1 GB di C++ per lo stesso eseguibile.

## Verifica

Test differenziale contro l'originale in Unicorn, usando direttamente la libreria dell'immagine
intera (`difftest.py --lib`):

| Libreria | Funzioni provate | Identiche | Diverse | Saltate |
|---|---|---|---|---|
| clang `-O1` | 8.000 | 4.199 | 0 | 3.801 |
| gcc `-O1` | 8.000 | 4.199 | 0 | 3.801 |
| clang `-O2` | **19.043** (tutte le candidate) | **9.892** | **0** | 9.151 |

Delle 9.892, 12 sono identiche a meno di un NaN o dell'esponente di un registro MMX
(`traduttore.md`). Due compilatori diversi danno gli stessi risultati: il bug di clang trovato
il giorno prima è aggirato, e non ne emergono altri in questo campione.

## Cosa questo non dice

- **Verificate sono 9.892 funzioni su 116.077**, più quelle che chiamano: l'8,5%. Sono quelle
  senza import, senza chiamate indirette e che girano con ingressi casuali. Il resto compila ma
  non è stato eseguito.
- **Niente è stato ancora eseguito "da gioco"**: manca il runtime Win32 (Fase 2). L'immagine
  intera oggi serve solo come libreria per il test.
- Prestazioni non misurate. Il codice legge e scrive i registri nel contesto a ogni istruzione.
- Restano i limiti della scoperta (`funzioni.md`): funzioni che non ritornano, tabelle delle
  eccezioni, flag nelle tail call tra funzioni spezzate.
