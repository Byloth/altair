# Strumenti

Tutti gli script leggono i file del gioco e non scrivono mai nella sua cartella.

```sh
export AC1="/mnt/e/Giochi/Steam/steamapps/common/Assassins Creed"
```

## `pe.py`

Analisi statica di PE32, solo libreria standard.

```sh
python3 -I tools/pe.py info    "$AC1/AssassinsCreed_Dx9.exe"   # hash, header, sezioni, import, RTTI, build string
python3 -I tools/pe.py imports "$AC1/AssassinsCreed_Dx9.exe"   # elenco funzioni importate per DLL
python3 -I tools/pe.py diff    "$AC1/AssassinsCreed_Dx9.exe" "$AC1/AssassinsCreed_Dx10.exe"
python3 -I tools/pe.py rtti    "$AC1/AssassinsCreed_Dx9.exe"   # nomi di tipo RTTI decorati
```

`rtti` è una scansione veloce di stringhe (4.376 nomi nella DX9). Per gerarchie e vtable
usa `rtti.py`. L'output contiene nomi di classi Ubisoft: se lo salvi, mettilo in `private/`.

## `rtti.py`

Parser dell'RTTI di MSVC a 32 bit: type descriptor, complete object locator, gerarchie
delle classi, vtable con i loro slot.

```sh
python3 -I tools/rtti.py "$AC1/AssassinsCreed_Dx9.exe"                               # statistiche
python3 -I tools/rtti.py "$AC1/AssassinsCreed_Dx9.exe" --out private/rtti-dx9.json   # dettaglio
```

Il JSON ha tre liste: `types` (indirizzo, nome decorato e leggibile), `hierarchies` (basi di
ogni classe con gli spiazzamenti) e `vtables` (indirizzo, classe, offset, slot). Risultati in
`docs/analisi/rtti.md`.

## `imports.py`

Mappa degli import: nome (ordinali risolti), cella IAT, riferimenti in `.text`, categoria.

```sh
python3 -I tools/imports.py "$AC1/AssassinsCreed_Dx9.exe"          # tabelle Markdown
python3 -I tools/imports.py "$AC1/AssassinsCreed_Dx9.exe" --guids  # GUID COM noti presenti
```

Le tabelle di `docs/analisi/import.md` vengono da qui.

## `functions.py`

Scoperta delle funzioni senza simboli (prototipo). Serve Capstone, quindi il Python del venv.

```sh
.venv/bin/python tools/functions.py "$AC1/AssassinsCreed_Dx9.exe" --out private/functions-dx9.json
.venv/bin/python tools/functions.py "$AC1/AssassinsCreed_Dx9.exe" --strict   # regola più severa
```

Stampa le statistiche in JSON; con `--out` salva l'elenco `[indirizzo, origine]`. Circa 30
secondi. Metodo, risultati e limiti in `docs/analisi/funzioni.md`.
