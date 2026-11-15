# Ambiente di lavoro

Macchina: Windows con WSL2, Ubuntu 24.04, 28 core, 31 GB di RAM. Si lavora in WSL; il gioco
sta sul disco Windows `E:` e si legge da `/mnt/e/...`.

## Installato a livello di sistema (dall'utente)

| Strumento | Versione | Note |
|---|---|---|
| clang, lld, llvm | 18.1.3 | compilano il C generato, anche per wasm32 |
| build-essential, cmake, pkg-config | cmake 3.28.3 | |
| OpenJDK | 25.0.5 | richiesto da Ghidra 12.2; il 21 è stato rimosso |
| python3-venv, python3-pip | Python 3.12.3 | |
| Rust (rustup) | 1.100.0, target `x86_64-unknown-linux-gnu` | in `~/.local/share/cargo` e `~/.local/share/rustup`; il PATH lo imposta `~/.bash_exports` |

**Nelle shell non interattive** (script, strumenti di Claude) `~/.bashrc` non carica
`~/.bash_exports`: va fatto `source ~/.bash_exports` prima di usare `cargo`.

## Dentro il progetto (ignorato da git)

| Percorso | Cosa | Come |
|---|---|---|
| `.venv/` | unicorn 2.1.4, capstone 5.0.7 | `python3 -m venv .venv && .venv/bin/pip install unicorn capstone` |
| `opt/ghidra_12.2_PUBLIC/` | Ghidra 12.2 (885 MB) | zip ufficiale, SHA-256 `2c2cd776…a7be67` verificato con quello della release |
| `ref/` | scimitar-recomp, EaglePatch | `git clone` |

## Grafica e audio in WSL

Niente da installare. Per usare la GPU (RTX 4070 Ti) da WSL:

```sh
export GALLIUM_DRIVER=d3d12 LD_LIBRARY_PATH=/usr/lib/wsl/lib
target/release/altair-gpu-probe      # deve elencare "Gl | D3D12 (NVIDIA GeForce RTX 4070 Ti)"
```

Senza queste variabili OpenGL e Vulkan usano la CPU (llvmpipe). Dettagli in
`analisi/d3d9.md`. Facoltativi, solo per diagnosi: `sudo apt install -y mesa-utils vulkan-tools`
(`glxinfo`, `vulkaninfo`).

## Avviare Ghidra

```sh
export JAVA_HOME=/usr/lib/jvm/java-25-openjdk-amd64
opt/ghidra_12.2_PUBLIC/ghidraRun                    # interfaccia grafica (WSLg)
opt/ghidra_12.2_PUBLIC/support/analyzeHeadless ...  # analisi da script
```

Senza `JAVA_HOME` e senza un terminale interattivo, Ghidra si ferma perché non può chiedere
il percorso del JDK.

## Spostare o rinominare la cartella del progetto

A sessione di Claude Code chiusa, da un terminale normale:

```sh
bash ~/projects/ac-rek/tools/migra-cartella.sh ~/projects/ac-rek ~/projects/altair
cd ~/projects/altair && claude --resume
```

Lo script sposta la cartella (con `ref/`, `opt/`, `private/`, `build/`), copia conversazioni e
memoria di Claude Code sotto il nuovo percorso (`~/.claude/projects/<percorso con i trattini>`),
ricrea il venv, che contiene percorsi assoluti, e ricompila. Gli originali delle conversazioni
restano al vecchio posto come copia di sicurezza.

## Compilare per Windows da WSL (2026-11-15)

Il runtime e il gioco tradotto si compilano per Windows **da WSL**, senza eseguire nulla di
Windows (nel `wsl.conf` dell'utente l'interoperabilità è spenta e i file sotto `/mnt/c` non
sono eseguibili: scelta rispettata).

Cosa serve:

| Cosa | Dove | Chi l'ha messo |
|---|---|---|
| LLVM-MinGW (UCRT) | Windows, `winget install MartinStorsjo.LLVM-MinGW.UCRT` | utente |
| Rust su Windows (`rustup`, toolchain GNU) | Windows | utente; **oggi non serve**, vedi sotto |
| `rust-std` 1.100.0 per `x86_64-pc-windows-gnullvm` | `opt/rust-win/` (ignorata da git) | scaricato da `static.rust-lang.org`, SHA-256 `5e63b95c…4384cf` verificato |

Come funziona:

- **Il C generato** lo compila il clang di Linux con `--target=x86_64-w64-mingw32` e, come
  libreria di sistema, la cartella di LLVM-MinGW letta da `/mnt/c` (`tools/build-image-win.sh`).
  Serve la variante MinGW e non quella di Visual Studio perché solo lì `long double` è a 80 bit
  (verificato: `sizeof(long double)` = 16), come vuole l'emulazione dell'x87.
- **Il runtime in Rust** si compila con il bersaglio `x86_64-pc-windows-gnullvm`. La sua
  libreria standard non è nell'installazione di Rust dell'utente: sta in `opt/rust-win/`, e
  `opt/rust-win/sysroot` è una cartella di collegamenti che unisce quella e la toolchain di
  Linux (`RUSTFLAGS=--sysroot=…`). Il collegamento finale lo fa `tools/win-link.sh`: clang di
  Linux, `lld`, librerie di LLVM-MinGW, `libunwind` statica.
- `tools/build-win.sh` fa le due cose. Risultato: `build/win/altair_ac1.dll` (63 MB) e
  `target/x86_64-pc-windows-gnullvm/release/altair-run.exe` (74 MB), che dipendono solo da DLL
  di sistema.

Per rifare `opt/rust-win/sysroot` dopo un aggiornamento di Rust: scaricare il `rust-std` della
stessa versione di `rustc --version`, e ricreare i collegamenti (comandi nel diario del
2026-11-15). Il Rust installato su Windows resta utile per compilare lì, ma questa strada non
lo usa.
