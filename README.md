<div align="center">

```
 ████████  ██████  ██████  ████████  ██████  ██████
    ██    ██    ██ ██   ██    ██    ██    ██ ██   ██
    ██    ██    ██ ██████     ██    ██    ██ ██████
    ██    ██    ██ ██         ██    ██    ██ ██
    ██     ██████  ██         ██     ██████  ██
```

### a gorgeous, feature‑rich terminal system monitor

**htop power · btop looks · written in Rust · one tiny binary**

[![CI](https://github.com/ur-grue/toptop/actions/workflows/ci.yml/badge.svg)](https://github.com/ur-grue/toptop/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.82%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)
![Platform](https://img.shields.io/badge/platform-linux-informational)
![Dependencies](https://img.shields.io/badge/runtime%20deps-0-success)

[Features](#-features) · [Install](#-install) · [Usage](#-usage) · [Keys](#-keybindings) · [Themes](#-themes) · [Architecture](#-architecture)

</div>

---

```text
▟▛ toptop  vm  Linux (Ubuntu 24.04)  kernel 6.18.5  up 00:12:43  load 0.26 0.22 0.15   10:00:04
╭ cpu ──────────────────────────────╮╭ memory ─────────────────╮╭ network ─────────────────╮
│all   1.6%  2.80 GHz  4c/4t        ││ram   3.8%  610 MiB / 15.││eth0  ↓ 1.2M/s     ↑ 40K/s│
│        ⢀⣀⣠⣄⣀⡀      ⢀⡀            ││█▍░░░░░░░░░░░░░░░░░░░░░░░░││Σ ↓2.0 MiB  ↑40.4 MiB     │
│ 0 ▎███░ 23  1 ██░░ 12             ││swp   0.0%  0 B / 0 B    ││   ⢀⣀⣠⣄⣀⡀   (rx)         │
│ 2 █░░░  8   3 ███░ 19             │╰─────────────────────────╯╰──────────────────────────╯
│        ⢀⣀⣠⣄⣀⡀      ⢀⡀            │╭ gpu · sensors ──────────╮╭ disk ────────────────────╮
│ ⢀⣀⣀⣠⣄⣀⣀⡀  (load gradient)        ││gpu0      ███▌ 41%  56°C ││io R 1.1M/s     W 0 B/s   │
│ ⠉⠛⠿⠟⠋⠉                            ││  RTX 4090 · 2G / 24G    ││   ⢀⣀⣠⣄⣀⡀  ⢀⡀  (r/w)     │
╰───────────────────────────────────╯╰─────────────────────────╯╰──────────────────────────╯
╭ processes · CPU% (▼) ────────────────────────────────────────────────────────────────────╮
│PID     USER      CPU%  MEM%  MEM     DISK     TIME    S COMMAND                            │
│    560 root        4.2   2.4 379M    1.1M/s   11:52   S claude --output-format=stream-json │
│    543 root        0.4   0.4  59M    ·        12:00   S environment-manager task-run       │
│  19201 you         2.7   0.0  12M    ·        00:16   R toptop                             │
╰──────────────────────────────────────────────────────────────────────────────────────────╯
?:help Enter:detail n:net s:sort t:tree /:filter K:signal L:layout q:quit
```

> A fast, beautiful, at‑a‑glance view of your machine — high‑resolution braille graphs,
> smooth truecolor gradient meters, an interactive process table with tree view and a
> one‑keystroke signal menu, GPU + sensors, a live network‑connections inspector, and five
> hand‑tuned themes — all in a single **~1 MB binary with zero runtime dependencies**.

## ✨ Features

| | |
|---|---|
| 📊 **High‑res braille graphs** | 2×4 dots per cell → 4× the detail of block graphs, with a vertical load gradient |
| 🌈 **Truecolor gradient meters** | CPU (global + per‑core), RAM, swap, disks, GPU and sensors |
| 🧠 **Interactive process table** | sort 6 ways, tree view, live filter, click‑to‑sort headers, full mouse support |
| 🔎 **Process detail view** | PPID, state, threads, RSS/virtual mem, **live disk I/O rates**, start time, exe, cwd |
| ☠️ **Signal menu** | send any of nine signals (`SIGTERM`…`SIGUSR2`) behind a confirmation prompt |
| 💾 **Per‑process I/O** | a live `DISK` column and sort key for read+write throughput |
| 🌐 **Network panel** | per‑interface rx/tx with a mirrored braille graph and totals |
| 🔌 **Connections inspector** | live TCP/UDP table mapping sockets → owning process (press `n`) |
| 🗄️ **Disk panel** | per‑mount usage meters plus a mirrored read/write I/O graph |
| 🎮 **GPU monitoring** | NVIDIA (`nvidia-smi`) **and** AMD/Intel (`sysfs`), polled off‑thread |
| 🤖 **AI / local‑LLM view** | compute **vs. memory‑bandwidth** util, VRAM spill warning, per‑process VRAM, throttle flag, and inference‑runtime detection (press `a`) |
| 🌡️ **Sensors & battery** | temperatures scaled to each critical threshold; battery in the header |
| 🎨 **Six themes** | `gruvbox` · `nord` · `dracula` · `tokyonight` · `matrix` · `cyberpunk`, cycle live with `p` |
| 🧩 **Saveable layouts** | `full` / `cpu` / `process` presets, cycled with `L` and persisted |
| 🕐 **Live header** | wall clock, uptime, load average, task counts, battery |
| 📐 **Adaptive layout** | reflows cleanly from a 250‑column desktop down to a tiny pane |
| 🪶 **Tiny & safe** | ~1 MB binary, no runtime deps, restores your terminal even on panic |
| 🤖 **Headless mode** | `--snapshot` prints a one‑shot textual report for scripts & dashboards |

## 🤖 For local‑LLM / AI engineers

Press **`a`** (or launch with **`--ai`**) for a view built around the question
*"why is my model slow?"* — the metrics a bare `nvidia-smi` doesn't make obvious:

- **Compute vs. memory‑bandwidth utilization.** Once a model fits in VRAM,
  token generation is almost always **bandwidth**‑bound, not compute‑bound — so
  toptop shows `utilization.memory` as its own meter next to core %. A GPU at
  "30% utilization" that's actually saturating memory bandwidth is your bottleneck.
- **VRAM headroom with a spill warning.** VRAM is a hard wall: cross it and your
  runtime offloads layers to system RAM for a **5–20× slowdown**. toptop colors
  the VRAM meter by pressure and warns *before* you spill.
- **Per‑process VRAM.** See exactly which PID is holding GPU memory — catch a
  model "squatting" on VRAM (`keep_alive`) or confirm your server is resident.
- **Power vs. limit + throttle flag.** Thermal/power throttling silently drops
  tokens/sec; toptop reads the driver's throttle reasons and flags it.
- **Inference‑runtime detection.** Ollama, llama.cpp, vLLM, SGLang, TGI,
  KoboldCpp, ExLlama, MLX, LocalAI and friends are recognized and listed with
  their CPU, RAM and VRAM — including **CPU‑only** inference when there's no GPU.

Pipe it anywhere with `toptop --export json` (includes GPU bandwidth/power/throttle
and GPU processes) — the building block for multi‑host fleet monitoring of an
inference cluster.

## 🚀 Install

**From source** (requires a Rust toolchain, 1.82+):

```bash
git clone https://github.com/ur-grue/toptop && cd toptop
cargo build --release
./target/release/toptop          # or: cargo install --path .
```

**Homebrew** (via tap):

```bash
brew install ur-grue/tap/toptop
# before a tagged release is published, install the tip of main:
brew install --HEAD ur-grue/tap/toptop
```

**Debian / Ubuntu** (`.deb`):

```bash
# grab toptop_*_amd64.deb from the GitHub Releases page, then:
sudo apt install ./toptop_1.0.0-1_amd64.deb
# build it yourself from a checkout:
cargo install cargo-deb && cargo deb     # writes target/debian/toptop_*.deb
```

> **Status:** `brew` and `apt` install from a published tap / release. Pushing a
> `v*` tag triggers [the release workflow](.github/workflows/release.yml), which
> builds the binary tarball and `.deb` and attaches them to the GitHub Release;
> the [Homebrew formula](packaging/homebrew/toptop.rb) is ready to drop into a tap.

### Platform support

`toptop` is **Linux-first**. The cross-platform core — CPU, memory, network,
disk and the process table (via `sysinfo`) — also builds and runs on macOS and
BSD; the Linux-specific panels (GPU via `sysfs`, battery, and the `/proc`-based
connections inspector) simply stay empty there rather than failing.

## 🎛️ Usage

```text
toptop [OPTIONS]

OPTIONS:
    -t, --tick <MS>      Refresh interval in milliseconds (100‑60000)
        --theme <NAME>   Color theme (gruvbox, nord, dracula, tokyonight, matrix, cyberpunk)
        --tree           Start in process‑tree view
        --no-tree        Start in flat process view
        --ai             Open the AI / local‑LLM GPU view on launch
        --list-themes    Print available themes and exit
        --snapshot       Print a one‑shot text snapshot and exit (no TUI)
        --export json    Print a machine‑readable JSON snapshot and exit
    -h, --help           Show help
    -V, --version        Show version
```

## ⌨️ Keybindings

| Key | Action | Key | Action |
|-----|--------|-----|--------|
| `↑`/`↓` `k`/`j` | move selection | `t` | toggle process tree |
| `PgUp`/`PgDn` | page up / down | `e` | toggle per‑core CPU meters |
| `Home`/`End` `g`/`G` | first / last | `a` | AI / local‑LLM GPU view |
| `Enter` | process detail view | `n` | network connections |
| `s` | cycle sort column | `L` | cycle layout preset |
| `i` | invert sort order | `p` / `P` | next / previous theme |
| click header | sort by column | `+` / `-` | faster / slower refresh |
| `/` | filter processes | `space` | pause / resume |
| `K` / `F9` | signal menu | `?` / `F1` | help overlay |
| `Del` | terminate (SIGTERM) | `x` | kill (SIGKILL) |
| `Esc` | close overlay / clear filter / quit | `q` / `Ctrl‑C` | quit |

## 🎨 Themes

Cycle live with `p` / `P`, or launch with `--theme <name>`:

`gruvbox` &nbsp;·&nbsp; `nord` &nbsp;·&nbsp; `dracula` &nbsp;·&nbsp; `tokyonight` &nbsp;·&nbsp; `matrix` &nbsp;·&nbsp; `cyberpunk`

Each theme defines a full semantic palette **and** a load gradient (green → yellow → red),
emitted as true 24‑bit RGB so meters and graphs interpolate smoothly on any modern terminal.

## 🧱 Architecture

`toptop` is a small, testable **library** (`src/lib.rs`) with a thin binary on top:

| Module | Responsibility |
|--------|----------------|
| `metrics` | `sysinfo` handles + time‑series histories; CPU/mem/net/disk/proc data |
| `metrics::gpu` | NVIDIA (`nvidia-smi`) + AMD/Intel (`sysfs`) GPU polling, off the UI thread |
| `metrics::netconn` | `/proc/net/*` parsing and socket→PID mapping for the connections view |
| `history` | fixed‑capacity ring buffer feeding the graphs |
| `theme` | themes and the truecolor gradient engine |
| `ui` + `ui::graph` | all rendering; braille‑graph and gradient‑meter primitives |
| `app` | state machine: input, sort/filter/tree, selection, signals, layout |
| `config` | dependency‑free config with optional persistence |

### Development

```bash
cargo test                          # unit + headless render integration tests
cargo clippy --all-targets          # lint (CI runs with -D warnings)
cargo run --example preview 120 40  # render one frame as plain text
cargo run -- --snapshot             # one-shot textual snapshot
```

The render tests drive the **full UI** through ratatui's headless `TestBackend` across
terminal sizes from 1×1 upward, so layout regressions and geometry panics are caught in CI
without a real terminal. Pure parsers (GPU, connections, gradients, formatting) are unit‑tested
directly.

## 🐚 Shell completions

Completion scripts for bash, zsh, and fish live in [`completions/`](completions/):

```bash
source completions/toptop.bash                    # bash (or copy to /etc/bash_completion.d/)
cp completions/_toptop ~/.zfunc/                  # zsh  (ensure ~/.zfunc is on your $fpath)
cp completions/toptop.fish ~/.config/fish/completions/   # fish
```

## 📦 Config

Settings live at `~/.config/toptop/config.conf` (honoring `XDG_CONFIG_HOME`) and are written
on exit, so your theme, refresh rate, tree mode and layout persist between runs. CLI flags
always override the file.

## 📝 License

[GPL‑3.0‑or‑later](LICENSE).

<div align="center">
<sub>Built with 🦀 Rust · ratatui · crossterm · sysinfo</sub>
</div>
