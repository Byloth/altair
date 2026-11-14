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

Nota: `rtti` conta i **nomi distinti** trovati nel file (4.043 nella DX9). scimitar-recomp
riporta 4.376 **descrittori**: alcuni nomi compaiono in più descrittori. Il conteggio esatto
dei descrittori arriverà con il parser RTTI vero (Fase 0).

L'output di `rtti` contiene nomi di classi Ubisoft: se lo salvi, mettilo in `private/`.
