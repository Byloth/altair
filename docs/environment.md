# Working environment

Machine: Windows with WSL2, Ubuntu 24.04, 28 cores, 31 GB of RAM. Work is done in WSL; the game
is on the Windows drive `E:` and is read from `/mnt/e/...`.

## Installed system-wide (by the user)

| Tool | Version | Notes |
|---|---|---|
| clang, lld, llvm | 18.1.3 | compile the generated C, also for wasm32 |
| build-essential, cmake, pkg-config | cmake 3.28.3 | |
| OpenJDK | 25.0.5 | required by Ghidra 12.2; version 21 was removed |
| python3-venv, python3-pip | Python 3.12.3 | |
| Rust (rustup) | 1.100.0, target `x86_64-unknown-linux-gnu` | in `~/.local/share/cargo` and `~/.local/share/rustup`; the PATH is set by `~/.bash_exports` |

**In non-interactive shells** (scripts, Claude's tools) `~/.bashrc` does not load
`~/.bash_exports`: run `source ~/.bash_exports` before using `cargo`.

## Inside the project (ignored by git)

| Path | What | How |
|---|---|---|
| `.venv/` | unicorn 2.1.4, capstone 5.0.7 | `python3 -m venv .venv && .venv/bin/pip install unicorn capstone` |
| `opt/ghidra_12.2_PUBLIC/` | Ghidra 12.2 (885 MB) | official zip, SHA-256 `2c2cd776…a7be67` verified against the release's |
| `ref/` | scimitar-recomp, EaglePatch | `git clone` |

## Graphics and audio in WSL

Nothing to install. To use the GPU (RTX 4070 Ti) from WSL:

```sh
export GALLIUM_DRIVER=d3d12 LD_LIBRARY_PATH=/usr/lib/wsl/lib
target/release/altair-gpu-probe      # must list "Gl | D3D12 (NVIDIA GeForce RTX 4070 Ti)"
```

Without these variables OpenGL and Vulkan use the CPU (llvmpipe). Details in
`analysis/d3d9.md`. Optional, for diagnosis only: `sudo apt install -y mesa-utils vulkan-tools`
(`glxinfo`, `vulkaninfo`).

## Starting Ghidra

```sh
export JAVA_HOME=/usr/lib/jvm/java-25-openjdk-amd64
opt/ghidra_12.2_PUBLIC/ghidraRun                    # graphical interface (WSLg)
opt/ghidra_12.2_PUBLIC/support/analyzeHeadless ...  # scripted analysis
```

Without `JAVA_HOME` and without an interactive terminal, Ghidra stops because it cannot ask
for the JDK path.

## Moving or renaming the project folder

With the Claude Code session closed, from a normal terminal:

```sh
bash ~/projects/ac-rek/tools/migrate-folder.sh ~/projects/ac-rek ~/projects/altair
cd ~/projects/altair && claude --resume
```

The script moves the folder (with `ref/`, `opt/`, `private/`, `build/`), copies Claude Code's
conversations and memory under the new path (`~/.claude/projects/<path with dashes>`),
recreates the venv, which contains absolute paths, and recompiles. The original conversations
stay in the old location as a backup.

## Building for Windows from WSL (2026-11-15)

The runtime and the translated game are built for Windows **from WSL**, without running anything
from Windows (in the user's `wsl.conf` interoperability is off and files under `/mnt/c` are
not executable: a choice we respect).

What is needed:

| What | Where | Who put it there |
|---|---|---|
| LLVM-MinGW (UCRT) | Windows, `winget install MartinStorsjo.LLVM-MinGW.UCRT` | user |
| Rust on Windows (`rustup`, GNU toolchain) | Windows | user; **not needed today**, see below |
| `rust-std` 1.100.0 for `x86_64-pc-windows-gnullvm` | `opt/rust-win/` (ignored by git) | downloaded from `static.rust-lang.org`, SHA-256 `5e63b95c…4384cf` verified |

How it works:

- **The generated C** is compiled by the Linux clang with `--target=x86_64-w64-mingw32` and, as
  the system library, the LLVM-MinGW folder read from `/mnt/c` (`tools/build-image-win.sh`).
  The MinGW variant is needed rather than the Visual Studio one because only there is `long double` 80 bits wide
  (verified: `sizeof(long double)` = 16), as the x87 emulation requires.
- **The Rust runtime** is built with the target `x86_64-pc-windows-gnullvm`. Its
  standard library is not in the user's Rust installation: it lives in `opt/rust-win/`, and
  `opt/rust-win/sysroot` is a folder of links that merges it with the Linux
  toolchain (`RUSTFLAGS=--sysroot=…`). The final link is done by `tools/win-link.sh`: Linux
  clang, `lld`, LLVM-MinGW libraries, static `libunwind`.
- The icon needs `llvm-rc` and `llvm-cvtres` (on Ubuntu they come with the `llvm` package,
  already present here in `/usr/bin`): they compile the extracted icon into a resource to link in.
- `tools/build-win.sh` does both. Result: `build/win/altair_ac1.dll` (63 MB) and
  `target/x86_64-pc-windows-gnullvm/release/altair-run.exe` (74 MB), which depend only on system
  DLLs.

To rebuild `opt/rust-win/sysroot` after a Rust update: download the `rust-std` of the
same version as `rustc --version`, and recreate the links (commands in the journal entry for
2026-11-15). The Rust installed on Windows remains useful for building there, but this route does
not use it.
