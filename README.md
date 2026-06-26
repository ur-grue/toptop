# toptop

**A gorgeous, feature-rich terminal system monitor — an htop / btop alternative built in Rust.**

`toptop` gives you a fast, beautiful, at-a-glance view of your machine: high‑resolution
braille graphs, smooth truecolor gradient meters, an interactive process table with tree
view and one‑keystroke `kill`, hardware sensors, and five hand‑tuned themes — all in a
single ~1 MB binary with no runtime dependencies.

```
▟▛ toptop  vm  Linux (Ubuntu 24.04)  kernel 6.18.5  up 00:20:44  load 0.29 0.28 0.18  109 tasks, 1 run
╭ cpu ──────────────────────────────╮╭ memory ──────────────╮╭ network ─────────────╮
│all   0.8%  2.80 GHz  4c/4t        ││ram   4.2%  668M/15.7G ││eth0  ↓ 1.2M/s ↑ 40K/s│
│        ⢀⣀⣠⣄⣀⡀     ⢀⡀             ││█▍░░░░░░░░░░░░░░░░░░░░░ ││   ⢀⣀⣠⣄⣀⡀  (rx)       │
│ 0 ▎███░  23   1 ██░░  12          ││swp   0.0%  0B / 0B    ││───────────────       │
│ 2 █░░░   8    3 ███░  19          ││░░░░░░░░░░░░░░░░░░░░░░░ ││   ⠉⠛⠿⠟⠋   (tx)       │
╰───────────────────────────────────╯╰───────────────────────╯╰───────────────────────╯
╭ processes · CPU% (▼) ─────────────────────────────────────────────────────────────────╮
│PID     USER      CPU%  MEM%  MEM     TIME    S COMMAND                                  │
│  19201 root        2.7   0.0 12M     00:16   S toptop                                   │
│    551 root        2.7   2.2 361M    19:59   S claude                                   │
╰─────────────────────────────────────────────────────────────────────────────────────╯
?:help s:sort t:tree /:filter K:kill p:theme space:pause q:quit
```

## Features

- **High‑resolution braille graphs** — 2×4 dots per cell means 4× the vertical and 2× the
  horizontal detail of block‑character graphs, with a vertical load gradient (calm at the
  base, hot at the peaks).
- **Truecolor gradient meters** for CPU (global + per‑core), RAM, swap, disks and sensors.
- **Interactive process table**
  - Sort by CPU, memory, PID, name, user, or runtime — ascending or descending.
  - **Tree view** showing the real parent/child process hierarchy.
  - Live **filter** as you type.
  - **Kill** the selected process — `SIGTERM` or `SIGKILL` — behind a confirmation prompt.
  - Full **mouse support**: click to select, scroll wheel to navigate.
- **Network** per‑interface throughput with a mirrored rx/tx braille graph and totals.
- **Disk** per‑mount usage meters plus aggregate read/write I/O rates.
- **Sensors** — temperatures scaled against each sensor's critical threshold.
- **Five themes** — `gruvbox`, `nord`, `dracula`, `tokyonight`, `matrix` — cycle live with `p`.
- **Adaptive layout** that reflows from a 250‑column desktop down to a tiny pane.
- **Config persistence** at `~/.config/toptop/config.conf`, plus CLI overrides.
- **Headless `--snapshot` mode** for scripts, dashboards, and machines without a TTY.
- Tiny, fast, dependency‑free binary; restores your terminal cleanly even on panic.

## Install

Requires a Rust toolchain (1.82+).

```bash
cargo build --release
./target/release/toptop
```

Or run directly during development:

```bash
cargo run --release
```

## Usage

```
toptop [OPTIONS]

OPTIONS:
    -t, --tick <MS>      Refresh interval in milliseconds (100-60000)
        --theme <NAME>   Color theme (gruvbox, nord, dracula, tokyonight, matrix)
        --tree           Start in process-tree view
        --no-tree        Start in flat process view
        --list-themes    Print available themes and exit
        --snapshot       Print a one-shot text snapshot and exit (no TUI)
    -h, --help           Show help
    -V, --version        Show version
```

### Keybindings

| Key | Action | Key | Action |
|-----|--------|-----|--------|
| `↑` / `↓`, `k` / `j` | move selection | `t` | toggle process tree |
| `PgUp` / `PgDn` | page up / down | `e` | toggle per‑core CPU meters |
| `Home`/`End`, `g`/`G` | first / last | `p` / `P` | next / previous theme |
| `s` | cycle sort column | `+` / `-` | faster / slower refresh |
| `i` | invert sort order | `space` | pause / resume |
| `/` | filter processes | `?` / `F1` | help overlay |
| `K` / `F9` / `Del` | terminate (SIGTERM) | `q` / `Ctrl‑C` | quit |
| `x` | kill (SIGKILL) | `Esc` | clear filter / quit |

## Architecture

`toptop` is organized as a small, testable library (`src/lib.rs`) with a thin binary on top:

| Module | Responsibility |
|--------|----------------|
| `metrics` | Owns the `sysinfo` handles and time‑series histories; produces all data. |
| `history` | Fixed‑capacity ring buffer for the graphs. |
| `theme` | Themes and the truecolor gradient engine. |
| `ui` + `ui::graph` | All rendering; the braille‑graph and gradient‑meter primitives. |
| `app` | State machine: input handling, sort/filter/tree, selection, kill flow. |
| `config` | Lightweight, dependency‑free config with optional persistence. |

### Development

```bash
cargo test                          # unit + headless render integration tests
cargo clippy --all-targets          # lint
cargo run --example preview 120 40  # render one frame as plain text
cargo run -- --snapshot             # one-shot textual snapshot
```

The render tests drive the full UI through ratatui's headless `TestBackend` across terminal
sizes from 1×1 upward, so layout regressions and geometry panics are caught in CI without a
real terminal.

## License

GPL‑3.0‑or‑later. See [LICENSE](LICENSE).
