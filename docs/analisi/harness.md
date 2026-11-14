# Harness Unicorn

`tools/harness.py` esegue funzioni dell'eseguibile **originale** in un emulatore x86 (Unicorn).
È l'oracolo dei test differenziali: quando il ricompilatore tradurrà una funzione, la stessa
chiamata con gli stessi ingressi dovrà dare gli stessi risultati qui e nel codice tradotto.

Verificato il 2026-11-14 sulla build Steam DX9 con Unicorn 2.1.4.

## Uso

```sh
.venv/bin/python tools/harness.py "$AC1/AssassinsCreed_Dx9.exe" selftest
.venv/bin/python tools/harness.py "$AC1/AssassinsCreed_Dx9.exe" call 0xFA6790 s:Altair
#   eax=0x00000006 edx=... istruzioni=30 import=[]
```

Opzioni di `call`: `--ecx V` (il `this` delle funzioni thiscall), `--trace` (stampa ogni
indirizzo eseguito). Gli argomenti sono interi oppure `s:testo`, copiato in memoria guest e
passato come puntatore.

Da Python:

```python
from harness import Guest
g = Guest(exe)
p = g.alloc_str(b'Altair')
r = g.call(0x00FA6790, [p])      # r.eax, r.edx, r.st0, r.popped, r.insns, r.imports
g.reset()                        # immagine e heap di prova tornano allo stato iniziale
```

`r.popped` è il numero di byte di argomenti tolti dallo stack dal chiamato: 0 per cdecl,
maggiore di 0 per stdcall e thiscall. Serve a scoprire la convenzione di chiamata.

## Come è fatto

| Indirizzo | Cosa |
|---|---|
| `0x00400000` | l'immagine PE, sezioni ai loro indirizzi (non servono rilocazioni) |
| `0x10000000` | heap di prova per gli argomenti, 1 MB, azzerato per intero a ogni `reset()` |
| `0x6FF00000`–`0x70000000` | stack, 1 MB |
| `0x7F000000` | trampolini degli import, 16 byte ciascuno |
| `0x7FFD0000` | TEB: `fs:[0]` fine della catena SEH, `fs:[0x18]` puntatore a sé, `fs:[0x2C]` array TLS |
| `0x7FFE0000` | GDT |
| `0x7FFF0000` | indirizzo di ritorno sentinella: arrivarci significa "la funzione è tornata" |

- **`fs:`** In modalità a 32 bit Unicorn non permette di scrivere direttamente la base di `fs`:
  serve una GDT con un descrittore la cui base è il TEB, e `fs` caricato con quel selettore.
- **Import.** Ogni cella della IAT punta a un trampolino. Quando l'esecuzione ci arriva, un hook
  cerca in `Guest.handlers` una funzione Python con quel nome, che riceve gli argomenti e
  restituisce `(eax, byte da togliere dallo stack)`. Se non c'è, la chiamata si ferma con
  `import non gestito: kernel32.dll!TlsGetValue (chiamato da 0x00fa8e1f)`. Tutte le chiamate
  agli import sono registrate in `r.imports`: nei test differenziali vanno confrontate anche
  quelle.
- **Limite di istruzioni** (5 milioni di default) contro i cicli infiniti.

## Prove automatiche (`selftest`)

Tre routine assembly del CRT di MSVC 8, trovate cercando in `.text` le loro sequenze iniziali
di byte e confrontate con il risultato calcolato in Python:

| Funzione | Indirizzo | Prova |
|---|---|---|
| `strlen` | `0x00FA6790` | 9 lunghezze × 4 allineamenti |
| `strcmp` | `0x00FA9760` | 9 coppie, segno del risultato |
| `memcpy` | `0x00F9FB80` | 49 lunghezze × 4 allineamenti della sorgente × 4 della destinazione: percorre le jump table di `memcpy`, uno dei punti delicati per il ricompilatore |

Tutte passano, in 0,4 secondi. Altre due chiamate di controllo:

- `0x007A56F0` (una funzione del motore con prologo SEH, che legge e scrive `fs:[0]`) ritorna
  correttamente: il TEB funziona.
- `_purecall` (`0x00F9F7A2`) si ferma su `TlsGetValue`, come atteso: chiama una funzione a
  `0x00FA8E10` (probabilmente `_decode_pointer` del CRT) che usa la TLS.

## Stato ripristinato a ogni chiamata

Imparato a spese del test differenziale (2026-11-15): l'emulatore conserva stato tra una
chiamata e l'altra, e una funzione interrotta a metà lo lascia sporco.

- **EFLAGS** viene riscritto a `0x202`: altrimenti il flag di direzione resta impostato.
- **MXCSR** viene reimpostato *eseguendo* un `ldmxcsr` in una paginetta di servizio: scrivere il
  registro dall'esterno non aggiorna arrotondamento, flush-to-zero e denormali interni.
- `reset()` ripristina sezioni scrivibili, tutto lo heap di prova e 64 KB di stack.
- `.text`, `.rdata` e `.rsrc` sono protette in scrittura: un puntatore sbagliato ferma il caso.
- I registri XMM iniziali si passano con `call(..., xmm=[...])`.

## Limiti e prossimi passi

- **Niente stato globale inizializzato.** L'immagine è quella su disco: gli inizializzatori
  statici del CRT e del motore non sono stati eseguiti. Vanno bene le funzioni pure; per le
  altre serve prima eseguire l'avvio (Fase 2) o preparare lo stato a mano.
- **Nessun gestore di import predefinito.** Si aggiungono man mano (`g.handlers['TlsGetValue'] = …`).
- **Un solo thread.**
- Per i test differenziali veri servono ancora: generazione degli ingressi, confronto di
  registri, stato x87 e memoria scritta, e una selezione di funzioni "foglia" (senza import,
  senza stato globale). È lavoro della Fase 1, insieme al ricompilatore.
- Unicorn (QEMU) differisce dall'hardware vero in due punti noti a scimitar-recomp: overflow di
  `fisttp` e conversioni di NaN segnalanti. `rcpps`/`rsqrtps` sono esatti in Unicorn e
  approssimati sulle CPU reali.

## Nota: la tabella CRC-32

In `.data` a `0x01915AC0` c'è la tabella CRC-32 standard (polinomio `0xEDB88320`), ma nessuna
istruzione e nessuna parola nei dati la referenzia direttamente. Il motore identifica le classi
negli archivi con il CRC-32 del nome, quindi una funzione che la usa deve esistere: va cercata
(accesso tramite un puntatore calcolato, o una seconda tabella generata a runtime). Sarebbe la
prima prova "del motore" da aggiungere al selftest: `CRC32("ShaderPermutation")` deve dare
`0x9467F2BB`.
