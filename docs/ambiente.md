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

## Avviare Ghidra

```sh
export JAVA_HOME=/usr/lib/jvm/java-25-openjdk-amd64
opt/ghidra_12.2_PUBLIC/ghidraRun                    # interfaccia grafica (WSLg)
opt/ghidra_12.2_PUBLIC/support/analyzeHeadless ...  # analisi da script
```

Senza `JAVA_HOME` e senza un terminale interattivo, Ghidra si ferma perché non può chiedere
il percorso del JDK.
