# Diario

## 2026-11-14

- Letto il documento di progetto "AC1 Recompiled: piano di progetto".
- Installata la build Steam in `E:\Giochi\Steam\steamapps\common\Assassins Creed`.
  La DX9 si blocca all'avvio; la DX10 funziona.
- Verificato lo SHA-1 della DX9 (`a9ea476e…432120`): coincide con quello che EaglePatch
  indica per Steam e GOG.
- Confrontate DX9 e DX10: stesso changelist 86610, DX9 con 4.043 nomi RTTI contro 937.
  → `analisi/eseguibili.md`, `decisioni.md`.
- Creati la struttura del repository, `tools/pe.py` e il piano di lavoro (`PIANO.md`).
- `git init`; clonati scimitar-recomp (`4f36655`) ed EaglePatch (`a39dc87`) in `ref/`, ignorata da git.
  Correzione: niente più cartelle fuori dal progetto.
- Letti `AC1_WINDOWS.md` e `PLAN.md` di scimitar-recomp → `analisi/scimitar-recomp.md`.
- **Scoperta:** la build Ubisoft Connect di scimitar-recomp ha lo stesso layout della nostra
  (entry, sezioni, TLS `0x1A3561C`, cookie `0x19CA838`, 178 export). I loro indirizzi valgono
  per noi → `decisioni.md`.
- Raccolti gli indirizzi DX9 di EaglePatch → `analisi/indirizzi-noti.md`.
- Toolchain da installare a cura dell'utente: Rust, clang/lld, build-essential, python3-venv,
  JDK 21, Ghidra 12.2 in `opt/`.
