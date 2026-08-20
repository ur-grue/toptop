# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **User-defined themes** — drop a `.conf` file into
  `$XDG_CONFIG_HOME/toptop/themes/` and its name joins the built-ins for
  `--theme`, `--list-themes` and the `p` cycle. `base = <built-in>` supplies
  every key the file doesn't set, colors are `#rrggbb`, and a broken file
  warns on stderr and is skipped rather than being fatal. (#17)
- **Configurable keybindings** — `bind_<action> = <key>[, <key>...]` in the
  config file remaps any main-view action (`bind_quit = ctrl+x`). Key names
  cover characters, arrows, `pgup`/`pgdn`, `home`/`end`, `enter`, `space`,
  `delete`, `f1`–`f12` and `ctrl+`/`alt+` prefixes; unknown keys warn and keep
  the default, and binding an already-used key moves it. `Esc` and `Ctrl-C`
  stay fixed. (#42)
- **Process-table column customization** — the `columns` config key chooses
  which columns are shown and in which order (`columns = pid, cpu, vram,
  command`). The header, the row cells and the click-to-sort mapping are all
  driven by the configured set. (#43)
- **Richer process detail** — the `Enter` overlay now also lists the selected
  process's **open files**, **network sockets** and **environment**, fetched
  lazily for just that PID while the overlay is open and dropped as soon as it
  closes. Each section is capped to its share of the panel and reports what it
  elided (`… 37 more`), so an elided list never looks like a complete one. The
  overlay now grows with the terminal instead of being a fixed 72×18 box.
  Environment comes from sysinfo (all platforms); open files and sockets are
  `/proc`-based and are simply empty elsewhere. (#44)

- **More inference runtimes detected** — TensorRT-LLM (`trtllm-serve`,
  Prometheus metrics incl. KV-cache utilization, queue and TTFT, plus
  tokens/sec from its counters), the `mlc-llm` launcher spelling (base MLC
  LLM detection landed in #54), and LM Studio (process detection plus the
  loaded model via its `/api/v0/models` endpoint). (#5)
- **TGI throughput and TTFT** — discovered Text Generation Inference servers
  now show generated and prefill tokens/sec (derived from TGI's cumulative
  per-request histogram sums) and a mean time-to-first-token estimated from
  its pipeline stages (validation + queue wait + prefill), alongside the
  existing batch-size and queue-depth gauges. (#10)
- **AI-view trend sparklines** — each discovered inference server now shows
  nvtop-style braille sparklines of tokens/sec (scaled to its own peak) and
  KV-cache % (scaled to 100) next to the instantaneous numbers, fed by
  per-server 256-sample histories that are pruned when a server goes
  away. (#30)
- `--config <path>` — use an explicit config file instead of
  `$XDG_CONFIG_HOME/toptop/config.conf`, and `--no-save` — don't write the
  config back on exit. A failed config save is now reported as a one-line
  stderr warning after the TUI shuts down instead of being silently
  swallowed. (#15)
- **Configurable alert thresholds** — the VRAM-spill, KV-cache-saturation and
  queue-backlog alert levels can now be tuned via `config.conf`
  (`alert_vram_pct`, `alert_kv_pct`, `alert_queue`) or CLI flags
  (`--alert-vram`, `--alert-kv`, `--alert-queue`). The thresholds apply to the
  TUI banner, `--export prometheus`, and the `--serve-metrics` endpoint. (#6)
- `--export csv` — the top processes as CSV (header row, RFC 4180 quoting) for
  spreadsheets and `awk`, alongside the existing `json` and `prometheus`
  exporters. (#16)

### Fixed

- Invalid command-line **values** (e.g. `--tick abc`, an unknown `--theme`)
  now exit with status 2 like other argument errors, as the man page always
  documented; they previously exited 1. (#19)

- **Signal delivery now reports a precise outcome** — *delivered*, *permission
  denied* (e.g. signalling another user's process without `sudo`), *already
  exited*, or *unsupported* — instead of a single generic "Failed to signal".
  The failure reason is read from the real `kill(2)` errno. (#20)
- **Signal-menu labels:** the confirmation prompt and status line now name all
  nine signals correctly; `SIGQUIT`, `SIGUSR1`, and `SIGUSR2` were previously
  mislabeled as `SIGTERM` or shown as a bare "signal". (#20)
- **Signal numbers** shown in the menu are now platform-correct on macOS
  (e.g. `SIGSTOP 17`, `SIGUSR1 30`) instead of hardcoded Linux values. (#20)
- The process table no longer refreshes while a kill confirmation is open, so
  the selected target can't vanish between prompt and confirm. (#20)

### Changed

- Battery is polled at most once every 10s instead of on every refresh tick,
  removing repeated `/sys` filesystem I/O from the hot path. (#20)
- A process's executable path and working directory are resolved on demand for
  the selected row only, rather than allocated for every process on every
  refresh. (#20)
- Internal: signal name and number now derive from a single authoritative
  `SIGNALS` table. (#21)

### Documentation

- Shell completions (bash/zsh/fish) now cover every flag — `--ai`, `--remote`,
  `--remote-cmd`, `--config`, `--no-save`, `--export`, `--serve-metrics` and
  the `--alert-*` family were missing — and the theme list includes
  `cyberpunk` and `paper`. Tests badge bumped to 113.
- Recolored the AI-view hero graphic to the default `gruvbox` theme so it
  matches what users see on first launch. (#24)
- Documented the signal-menu permission-denied behavior. (#23)
- Added macOS to the platform badge. (#25)
- Bumped the tests badge to 68. (#22)

## [1.0.1] — 2026-07-06

### Added

- `paper` theme for light terminals.
- macOS marked as a verified platform.

### Changed

- The repository is now its own Homebrew tap (`Formula/toptop.rb`); documented
  the third-party-tap trust step.

## [1.0.0] — 2026-07-06

Initial public release.

[Unreleased]: https://github.com/ur-grue/toptop/compare/v1.0.1...HEAD
[1.0.1]: https://github.com/ur-grue/toptop/compare/v1.0.0...v1.0.1
[1.0.0]: https://github.com/ur-grue/toptop/releases/tag/v1.0.0
