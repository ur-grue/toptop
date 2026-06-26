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
| 🌡️ **Sensors & battery** | temperatures scaled to each critical threshold; battery in the header |
| 🎨 **Five themes** | `gruvbox` · `nord` · `dracula` · `tokyonight` · `matrix`, cycle live with `p` |
| 🧩 **Saveable layouts** | `full` / `cpu` / `process` presets, cycled with `L` and persisted |
| 🕐 **Live header** | wall clock, uptime, load average, task counts, battery |
| 📐 **Adaptive layout** | reflows cleanly from a 250‑column desktop down to a tiny pane |
| 🪶 **Tiny & safe** | ~1 MB binary, no runtime deps, restores your terminal even on panic |
| 🤖 **Headless mode** | `--snapshot` prints a one‑shot textual report for scripts & dashboards |

## 🚀 Install

Requires a Rust toolchain (1.82+).

```bash
git clone https://github.com/ur-grue/toptop && cd toptop
cargo build --release
./target/release/toptop
```

Or run directly during development:

```bash
cargo run --release
```

## 🎛️ Usage

```text
toptop [OPTIONS]

OPTIONS:
    -t, --tick <MS>      Refresh interval in milliseconds (100‑60000)
        --theme <NAME>   Color theme (gruvbox, nord, dracula, tokyonight, matrix)
        --tree           Start in process‑tree view
        --no-tree        Start in flat process view
        --list-themes    Print available themes and exit
        --snapshot       Print a one‑shot text snapshot and exit (no TUI)
    -h, --help           Show help
    -V, --version        Show version
```

## ⌨️ Keybindings

| Key | Action | Key | Action |
|-----|--------|-----|--------|
| `↑`/`↓` `k`/`j` | move selection | `t` | toggle process tree |
| `PgUp`/`PgDn` | page up / down | `e` | toggle per‑core CPU meters |
| `Home`/`End` `g`/`G` | first / last | `n` | network connections |
| `Enter` | process detail view | `L` | cycle layout preset |
| `s` | cycle sort column | `p` / `P` | next / previous theme |
| `i` | invert sort order | `+` / `-` | faster / slower refresh |
| click header | sort by column | `space` | pause / resume |
| `/` | filter processes | `?` / `F1` | help overlay |
| `K` / `F9` | signal menu | `Del` | terminate (SIGTERM) |
| `x` | kill (SIGKILL) | `q` / `Ctrl‑C` | quit |
| `Esc` | close overlay / clear filter / quit | | |

## 🎨 Themes

Cycle live with `p` / `P`, or launch with `--theme <name>`:

`gruvbox` &nbsp;·&nbsp; `nord` &nbsp;·&nbsp; `dracula` &nbsp;·&nbsp; `tokyonight` &nbsp;·&nbsp; `matrix`

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
