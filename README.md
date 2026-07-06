<div align="center">

<img src="assets/banner.svg" alt="toptop — system monitor" width="100%">

<br>

### the local‑inference observability layer for your terminal

**Live tokens/sec · VRAM‑spill & throttle warnings · the GPU metrics `nvidia-smi` can't show —
with a gorgeous full system monitor underneath. Rust · one tiny binary · zero deps.**

[![CI](https://github.com/ur-grue/toptop/actions/workflows/ci.yml/badge.svg)](https://github.com/ur-grue/toptop/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/rust-1.82%2B-orange?logo=rust)](https://www.rust-lang.org)
[![License: GPL v3](https://img.shields.io/badge/license-GPLv3-blue.svg)](LICENSE)
![Platform](https://img.shields.io/badge/platform-linux-informational)
![Dependencies](https://img.shields.io/badge/runtime%20deps-0-success)
![Tests](https://img.shields.io/badge/tests-65%20green-success)

[AI view](#-for-ai-engineers) · [Fleet](#-multi-host-fleet-view) · [Prometheus](#-export--observability) · [Install](#-install) · [Features](#-features) · [Themes](#-themes)

</div>

---

**The local‑inference view (press `a`) — the numbers `nvidia-smi` doesn't show you:**

<img src="assets/ai-demo.svg" alt="toptop AI view — live tokens/sec, the memory-bandwidth bar pulsing, VRAM filling to red, spill alert firing" width="100%">

<details>
<summary>📄 text version (what it looks like in your terminal)</summary>

```text
╭ AI · local-LLM GPU view · Esc/a to close ──────────────────────────────────╮
│⚠ ALERTS                                                                    │
│  [warn] gpu0 VRAM 92% — risk of spilling to RAM                            │
│                                                                            │
│gpu0  NVIDIA GeForce RTX 4090   290/450W  72°C                              │
│  compute   █████████▎░░░░░░░░░░░░░░░░░░░░  31%                             │
│  mem b/w   ███████████████████████▍░░░░░░  78%                             │
│  vram      ███████████████████████████▌░░ 22.0 GiB / 24.0 GiB              │
│             ⚠ near VRAM limit — models may spill to RAM (5–20× slower)     │
│                                                                            │
│Inference servers (auto-discovered)                                         │
│  vLLM :8000  meta-llama/Llama-3-8B                                         │
│    83.4 tok/s (0.29 tok/s/W)  prefill 1240/s  kv 64%  req 2/5  ttft 180ms  │
│                                                                            │
│GPU processes (by VRAM)                                                     │
│      PID       VRAM   CPU%  PROCESS                                        │
╰────────────────────────────────────────────────────────────────────────────╯
```
</details>

> Local LLMs are **memory‑bandwidth bound** once the model is resident, so a GPU at "31% util" can
> still be your bottleneck. toptop shows **compute vs. bandwidth** side by side, warns **before**
> VRAM spills to system RAM (a 5–20× slowdown), reads the driver's throttle reasons, and
> **auto‑discovers your inference servers** (vLLM · llama.cpp · Ollama · TGI) to scrape live
> **tokens/sec**, KV‑cache pressure and queue depth — then exports it as JSON **or Prometheus**
> for Grafana, or fans out across a cluster with the [fleet view](#-multi-host-fleet-view).

<details open>
<summary><b>🖥️ …and a gorgeous full system monitor</b> &nbsp;(the default view)</summary>

```text
▟▛ toptop  vm  Linux (Ubuntu 24.04)  kernel 6.18.5  up 00:03:50  load 0.34 0.39 0.18  111 tasks, 1 run
╭ cpu ─────────────────────────────────────╮╭ memory ───────────────────────╮╭ network ──────────────────────╮
│all   2.4%  2.80 GHz  4c/4t               ││ram   3.8%  608 MiB / 15.7 GiB ││eth0  ↓ 0 B/s      ↑ 0 B/s     │
│                                          ││█▏░░░░░░░░░░░░░░░░░░░░░░░░░░░░░││Σ ↓1015 KiB  ↑11.4 MiB         │
│                                          ││swp   0.0%  0 B / 0 B          ││                               │
│                                          │╰───────────────────────────────╯╰───────────────────────────────╯
│                                        ⢠ │╭ sensors ──────────────────────╮╭ disk ─────────────────────────╮
│ 0 ░░░░░░   0  1 ░░░░░░   0               ││no sensors detected            ││io R 0 B/s      W 0 B/s        │
│ 2 ▎░░░░░   3  3 ▎░░░░░   3               ││                               ││/       ██████▎  89% 252G      │
╰──────────────────────────────────────────╯╰───────────────────────────────╯╰───────────────────────────────╯
╭ processes · CPU% (▼) ──────────────────────────────────────────────────────────────────────────────────────╮
│PID     USER      CPU%  MEM%  MEM     DISK     TIME    S COMMAND                                            │
│   3715 root        6.3   0.0 5.8M    ·        00:01   R target/debug/examples/preview 110 18               │
│    553 root        3.1   2.3 376M    ·        02:55   S claude --output-format=stream-json --verbose --sett│
│    568 root        0.0   2.3 376M    ·        02:52   S claude --output-format=stream-json --verbose --sett│
│    557 root        0.0   0.0 0       ·        02:54   I kworker/2:1H-kblockd                               │
╰────────────────────────────────────────────────────────────────────────────────────────────────────────────╯
?:help a:ai Enter:detail n:net s:sort t:tree /:filter K:signal L:layout q:quit
```

High‑resolution braille graphs, truecolor gradient meters, an interactive process table (tree,
filter, click‑to‑sort, signal menu), per‑interface network, per‑mount disk + I/O, temperature
sensors, battery, six themes — all in a single **~1 MB binary with zero runtime dependencies**.
</details>

<details>
<summary><b>🛰️ Multi‑host fleet view</b> &nbsp;(<code>--remote</code>) — monitor a whole inference cluster</summary>

```text
▟▛ toptop fleet  4 hosts · 3 online  · 6 GPUs · Σ vram 74G/184G  · Σ 154 tok/s              11:13:20
╭ hosts ───────────────────────────────────────────────────────────────────────────────────────────╮
│HOST             STATUS           CPU%  MEM   LOAD   GPU%  VRAM     TOK/S   TASKS  UP             │
│gpu-node-1       ● online         37    53    4.6    91    41G      58      512    1d 00:00:00    │
│gpu-node-2       ● online         88    91    11.0   22    12G      12      512    1d 00:00:00    │
│inference-3      ● online         12    28    1.5    76    21G      83      512    1d 00:00:00    │
│offline-box      ✕ ssh: connect:… —     —     —      —     —        —       —      —              │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
```
</details>

## ✨ Features

**For local inference / LLMOps**

| | |
|---|---|
| 🤖 **AI / local‑LLM view** | compute **vs. memory‑bandwidth** util, VRAM spill warning, throttle flag, per‑process VRAM, tokens/sec/watt, serving‑vs‑training detection (press `a`) |
| 🔭 **Server auto‑discovery** | finds listening vLLM / llama.cpp / Ollama / TGI and scrapes live **tokens/sec**, KV‑cache, queue depth, TTFT — no config |
| 🎮 **Multi‑vendor GPU** | NVIDIA (`nvidia-smi`) **and** AMD/Intel (`sysfs`), polled off‑thread — util, bandwidth, VRAM, power, temp, throttle |
| 📡 **Prometheus exporter** | `--serve-metrics` runs a `/metrics` endpoint (or `--export prometheus`) — scrape tokens/sec, VRAM, throttle into Grafana |
| 🚨 **Threshold alerts** | VRAM‑spill, GPU‑throttle, KV‑cache and queue‑backlog alerts — a red TUI banner **and** `toptop_alert` gauges for Alertmanager |
| 🛰️ **Multi‑host fleet** | `--remote h1,h2,…` aggregates a cluster over SSH — Σ tokens/sec, Σ VRAM, per‑host status |

**As a general system monitor**

| | |
|---|---|
| 📊 **High‑res braille graphs** | 2×4 dots per cell → 4× the detail of block graphs, with a vertical load gradient |
| 🌈 **Truecolor gradient meters** | CPU (global + per‑core), RAM, swap, disks, GPU and sensors |
| 🧠 **Interactive process table** | sort 6 ways, tree view, live filter, click‑to‑sort headers, full mouse support |
| 🔎 **Process detail · I/O** | PPID, threads, RSS/virtual mem, **live disk I/O rates**, exe, cwd; a sortable `DISK` column |
| ☠️ **Signal menu** | send any of nine signals (`SIGTERM`…`SIGUSR2`) behind a confirmation prompt |
| 🌐 **Network + connections** | per‑interface rx/tx braille graph; a live TCP/UDP table mapping sockets → process (`n`) |
| 🗄️ **Disk + sensors** | per‑mount usage, read/write I/O graph, temperatures, battery |
| 🎨 **Six themes & layouts** | `gruvbox` · `nord` · `dracula` · `tokyonight` · `matrix` · `cyberpunk`; `full`/`cpu`/`process` presets |
| 🪶 **Tiny & safe** | ~1 MB binary, zero runtime deps, adaptive 250‑col→tiny layout, restores your terminal even on panic |

## 🤖 For AI engineers

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

### …and the numbers `nvidia-smi` can't see

toptop already knows which PIDs are inference runtimes **and which ports they
listen on**, so it auto‑discovers your local servers and scrapes their metrics
endpoints — no config, no exporters:

- **Live tokens/sec** (generation *and* prefill), **KV‑cache pressure**, **queue
  depth** and **TTFT**, read straight from llama.cpp / vLLM / TGI Prometheus
  `/metrics` and Ollama `/api/ps` — over a tiny dependency‑free localhost client
  on a background thread.
- **tokens/sec/watt** — throughput fused with GPU power draw, the efficiency
  number that maps to your electricity/cloud bill.
- **Serving vs. training** — training launchers (torchrun, DeepSpeed, Axolotl,
  Unsloth, LLaMA‑Factory, Megatron, torchtune…) are detected too, with a
  **dataloader‑bound** hint when a run pins a CPU while the GPU sits idle.
- **Multi‑GPU aggregate** — Σ VRAM and Σ power across cards, with a **sharding
  imbalance** warning when one GPU is hot and the others idle.

```text
Inference servers (auto-discovered)
  vLLM :8000  meta-llama/Llama-3-8B
    83.4 tok/s (0.29 tok/s/W)  prefill 1240/s  kv 64%  req 2/5  ttft 180ms
```

## 📡 Export & observability

toptop isn't just a live dashboard — it's a metrics **source**. Emit a snapshot as
machine‑readable JSON or **Prometheus** exposition text and scrape it into Grafana /
VictoriaMetrics to chart tokens/sec, VRAM pressure, throttle events and queue depth over time:

```bash
toptop --export json          # one-shot snapshot (GPU bandwidth/power + server tokens/sec)
toptop --export prometheus    # one-shot Prometheus text
toptop --serve-metrics        # long-running /metrics endpoint on 127.0.0.1:9709
toptop --serve-metrics 0.0.0.0:9709   # bind elsewhere
```

```text
# TYPE toptop_gpu_mem_bandwidth_percent gauge
toptop_gpu_mem_bandwidth_percent{gpu="0",name="NVIDIA GeForce RTX 4090"} 78.00
# TYPE toptop_inference_tokens_per_second gauge
toptop_inference_tokens_per_second{runtime="vLLM",port="8000",model="meta-llama/Llama-3-8B"} 83.40
# TYPE toptop_alert gauge
toptop_alert{key="vram_spill",detail="gpu0",level="warn"} 1
```

**Threshold alerts** fire on the things that actually hurt local inference — VRAM about to
spill, GPU throttling, KV‑cache saturation, request‑queue backlog. They show as a red banner in
the TUI (top of the `a` view) **and** as `toptop_alert{…}` gauges, so you can page on them with
Prometheus Alertmanager. The same JSON powers the multi‑host fleet view below.

## 🔒 What it accesses

No surprises for serious infra: toptop is **read‑only**, **local‑first**, and has **no daemon
and no telemetry**. GPU stats come from `nvidia-smi` / `sysfs`; inference metrics from a
**localhost‑only** HTTP scrape of servers you're already running; the fleet view shells out to
**`ssh` using your own keys/agent** (`BatchMode`, never prompting). Nothing is sent anywhere you
don't point it at.

## 🛰️ Multi-host fleet view

Point toptop at a list of machines and it becomes a **local‑cluster dashboard** —
each host is polled over SSH (it just runs `toptop --export json` there), parsed,
and aggregated:

```bash
toptop --remote gpu-node-1,gpu-node-2,inference-3      # SSH hosts (and 'local')
toptop --remote-cmd "/opt/bin/toptop --export json" --remote box-a,box-b
```

```text
▟▛ toptop fleet  4 hosts · 3 online  · 6 GPUs · Σ vram 74G/184G  · Σ 154 tok/s              11:13:20
╭ hosts ───────────────────────────────────────────────────────────────────────────────────────────╮
│HOST             STATUS           CPU%  MEM   LOAD   GPU%  VRAM     TOK/S   TASKS  UP             │
│gpu-node-1       ● online         37    53    4.6    91    41G      58      512    1d 00:00:00    │
│gpu-node-2       ● online         88    91    11.0   22    12G      12      512    1d 00:00:00    │
│inference-3      ● online         12    28    1.5    76    21G      83      512    1d 00:00:00    │
│offline-box      ✕ ssh: connect:… —     —     —      —     —        —       —      —              │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ host · gpu-node-1 ───────────────────────────────────────────────────────────────────────────────╮
│Linux (Ubuntu 24.04)   load 4.62 3.70 3.08   up 1d 00:00:00   512 tasks (4 run)   18ms            │
│cpu   ███████████▏░░░░░░░░░░░░░░░░░░ 37%                                                          │
│mem   ████████████████░░░░░░░░░░░░░░ 34.0 GiB / 64.0 GiB                                          │
│vram  ███████████████▍░░░░░░░░░░░░░░ 41.0 GiB / 80.0 GiB  ·  2 GPU  ·  620 W                      │
│  vLLM Llama-3-70B  58.3 tok/s  kv 68%                                                            │
│                                                                                                  │
│                                                                                                  │
│                                                                                                  │
│                                                                                                  │
╰──────────────────────────────────────────────────────────────────────────────────────────────────╯
↑/↓:select p:theme q:quit
```

Aggregate **Σ tokens/sec** and **Σ VRAM** across the fleet; select a host to see its
CPU/MEM/VRAM meters and per‑server tokens/sec. Offline hosts show the SSH error inline
and keep retrying. No agent, no daemon, no open ports — just SSH and the single binary.

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
        --remote <HOSTS> Multi‑host fleet view (comma‑separated SSH hosts; 'local')
        --remote-cmd <C> Command run on each remote (default: toptop --export json)
        --list-themes    Print available themes and exit
        --snapshot       Print a one‑shot text snapshot and exit (no TUI)
        --export <FMT>   Print metrics and exit: 'json' (default) or 'prometheus'
        --serve-metrics [ADDR]  Run a Prometheus /metrics endpoint (default 127.0.0.1:9709)
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
| `metrics::ai` | local‑AI workload taxonomy (serving vs. training runtimes) |
| `metrics::infer` | inference‑server discovery + Prometheus/Ollama scraping (tokens/sec) |
| `metrics::netconn` | `/proc/net/*` parsing and socket→PID mapping |
| `alerts` | threshold rules (VRAM spill / throttle / KV / queue) → firing alerts |
| `export` + `serve` | JSON & Prometheus serializers + the `/metrics` HTTP endpoint |
| `json` + `fleet` | dependency‑free JSON parser + multi‑host SSH aggregation |
| `history` | fixed‑capacity ring buffer feeding the graphs |
| `theme` | themes and the truecolor gradient engine |
| `ui` + `ui::graph` + `ui::fleet` | all rendering; braille‑graph and gradient‑meter primitives |
| `app` | state machine: input, sort/filter/tree, selection, signals, layout |
| `config` | dependency‑free config with optional persistence |

### Development

```bash
cargo test                          # unit + headless render integration tests
cargo clippy --all-targets          # lint (CI runs with -D warnings)
cargo run --example preview 120 40  # render one frame as plain text
cargo run --example fleet_preview   # render the fleet dashboard with sample hosts
cargo run -- --snapshot             # one-shot textual snapshot
python3 scripts/make_banner.py      # regenerate the pixel-art banner (assets/banner.svg)
python3 scripts/make_ai_demo.py     # regenerate the animated AI-view demo (assets/ai-demo.svg)
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

## 🤝 Contributing

Bug reports, runtime integrations (MLC? TensorRT‑LLM?), and packaging help are all
welcome — see [CONTRIBUTING.md](CONTRIBUTING.md). The whole test suite runs headless
(`cargo test`), so you don't even need a GPU to hack on most of it.

**If toptop showed you why your model was slow, [a star](https://github.com/ur-grue/toptop/stargazers)
helps other people find it.** ⭐

## 📝 License

[GPL‑3.0‑or‑later](LICENSE).

<div align="center">
<sub>Built with 🦀 Rust · ratatui · crossterm · sysinfo</sub>
</div>
