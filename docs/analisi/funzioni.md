# Primo elenco di funzioni

Verificato il 2026-11-14 con `tools/functions.py` (prototipo in Python con Capstone) su
`AssassinsCreed_Dx9.exe`. L'elenco completo, con l'origine di ogni partenza, è in
`private/functions-dx9.json` (30 secondi per rigenerarlo):

```sh
.venv/bin/python tools/functions.py "$AC1/AssassinsCreed_Dx9.exe" --out private/functions-dx9.json
```

È una stima di lavoro, non l'elenco definitivo: quello lo produrrà il ricompilatore in Rust
(Fase 1), che dovrà anche risolvere i limiti elencati in fondo.

## Risultato

| | Regola larga (predefinita) | `--strict` | scimitar-recomp |
|---|---|---|---|
| Funzioni | **115.807** | 96.781 | 98.470 |
| Istruzioni raggiunte (distinte) | **3.191.741** | 3.121.093 | 3.484.935 |
| Jump table | 570 | 567 | 728 (conteggio da sweep lineare) |

I numeri non sono confrontabili uno a uno con quelli di scimitar-recomp: loro contano le
istruzioni per funzione (il codice condiviso tra due funzioni conta due volte), noi le istruzioni
distinte. L'ordine di grandezza coincide.

Le due regole differiscono su quando una parola nei dati, o un immediato, "sembra un inizio":

- `--strict`: indirizzo allineato a 16, oppure subito dopo padding (`int3`, `nop`) o `ret`;
- larga: anche subito dopo un `jmp`. Aggiunge 19.000 partenze ma solo 70.000 istruzioni: sono
  quasi tutte piccoli pezzi della gestione delle eccezioni (thunk `mov eax, tabella; jmp
  __CxxFrameHandler`, funclet di unwind e blocchi `catch`), raggiunti solo dal runtime delle
  eccezioni. Sono punti d'ingresso veri, ma servono solo se il gioco lancia eccezioni.

## Da dove vengono le partenze (regola larga)

| Origine | Funzioni |
|---|---|
| Entry point e ciò che chiama, direttamente o a catena | 7.116 |
| Export (motore audio DARE) e ciò che chiamano | 100 |
| Slot di vtable | 28.304 |
| Chiamate dirette dalle funzioni delle vtable | 23.400 |
| Parole nei dati | 36.254 (+3.401 chiamate da queste) |
| Immediati nel codice (`push offset f`) | 16.087 (+1.145) |

**Il fatto più importante: dall'entry point, seguendo solo le chiamate dirette, si raggiungono
7.216 funzioni su oltre 100.000.** Tutto il resto del gioco è raggiunto tramite vtable e
puntatori a funzione. Non si può tradurre "solo quello che serve" partendo da `main`: va
tradotta l'immagine intera, come ha concluso anche scimitar-recomp.

Dopo entry, export e vtable (le fonti più affidabili) si hanno già 58.920 funzioni e 2,5 milioni
di istruzioni: il 78% del codice raggiunto.

## Com'è fatta `.text` (19,4 MB)

| Contenuto | Byte | Quota |
|---|---|---|
| Codice raggiunto | 10.197.343 | 52,6% |
| Jump table dentro `.text` | 20.024 | 0,1% |
| `int3` non raggiunti (padding e il blocco morto da 3,5 MB) | 5.440.558 | 28,1% |
| Non classificato | 3.720.251 | 19,2% |

Il conteggio degli `int3` (5,44 milioni) coincide con quello di scimitar-recomp.

**I 3,7 MB non classificati sono in gran parte codice vero che nessuno referenzia.** Sono
27.000 blocchi, di cui 22.000 iniziano a un indirizzo allineato a 16 dopo padding e si
decodificano come funzioni normali (prologhi `push esi; mov esi, ecx`…). Né una chiamata, né
una vtable, né una parola nei dati punta a loro: codice morto che il linker non ha scartato.
Torna con i conti di scimitar-recomp: circa 4,4 milioni di istruzioni reali in `.text`, di cui
3,5 raggiunte. Non vanno tradotti, ma vanno tenuti d'occhio: se a runtime una chiamata indiretta
punta lì dentro, è una funzione che la scoperta ha perso.

## Controlli

- Tutti gli indirizzi noti da EaglePatch sono nell'elenco: `PadProxyPC::Update` (vtable),
  `PadProxyPC::AddPad`, l'allocatore e `getNewDescriptor` (dall'entry), il costruttore di
  `PadXenon` e il `delete` del motore (da immediati), `Pad::UpdateTimeStamps` (da vtable).
- Solo **13 slot di vtable su 28.717** non si decodificano come funzione: l'euristica con cui
  `rtti.py` conta gli slot è affidabile.

## Scarti e anomalie

| Motivo dello scarto di una partenza debole | Quante |
|---|---|
| Cade dentro un'istruzione o una tabella già note | 41.253 |
| Non sembra un inizio | 36.496 |
| Decodifica in istruzioni che un gioco non esegue (`in`, `out`, `les`, `retf`…) | 12.529 |
| Jump table sovrapposta a codice | 534 |
| Sovrapposizione con altre istruzioni | 184 |
| Byte non decodificabili | 44 |
| Salto o chiamata fuori da `.text` | 20 |

È l'effetto dell'assenza di rilocazioni: decine di migliaia di parole nei dati cadono per caso
nel range del codice e vanno filtrate.

Anomalie nelle partenze **forti** (codice sicuramente vero), da risolvere in Fase 1:

- 10 jump table che si sovrappongono a codice già decodificato e 2 sovrapposizioni: la lettura
  "golosa" delle tabelle va oltre la fine, oppure si è decodificato oltre una chiamata che non
  ritorna;
- 3 istruzioni improbabili e 1 atterraggio dentro un'istruzione: stessa causa probabile, il
  codice dopo una `call` a una funzione che non ritorna (`exit`, `_amsg_exit`) non è codice.

## Limiti del prototipo

1. **Jump table**: riconosce solo `jmp [reg*4 + T]` e legge voci finché sono indirizzi di
   codice, senza usare il limite del `cmp` che precede. Mancano le forme a due livelli con
   limite esplicito e gli indici negativi di `memcpy`.
2. **Funzioni che non ritornano**: non sono riconosciute, quindi si decodifica anche ciò che
   segue la chiamata.
3. **Nessun confine di funzione**: si sa dove una funzione inizia, non quali istruzioni le
   appartengono. Il ricompilatore dovrà calcolarlo (codice condiviso, tail call).
4. **Punti d'ingresso in codice già raggiunto** contati come funzioni: in parte sono etichette
   interne (blocchi `catch`), non funzioni chiamabili.
5. Non usa le tabelle delle eccezioni C++ (`FuncInfo`, unwind map), che darebbero funclet e
   handler in modo certo anziché euristico.
