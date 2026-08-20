# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **KV-cache preemption surfaced as a critical alert** — when a serving runtime
  runs out of KV cache it preempts in-flight requests, throwing away work
  already done and recomputing it later. Throughput collapses while every
  dashboard still shows a busy, healthy GPU. toptop scrapes vLLM's and
  TensorRT-LLM's preemption counters, differences them into a rate, shows
  `⟲ preempt 1.8/s` in the AI view, and alerts at **critical** — above a queue
  backlog, because a backlog only delays work whereas preemption destroys it.
  Tunable via `alert_preempt` / `--alert-preempt`, exported as
  `toptop_inference_preemptions_{total,per_second}`. (#31)
- **GPU compute-vs-bandwidth history** — per-GPU histories of compute,
  memory-bandwidth and VRAM utilization, drawn in the AI view as one mirrored
  braille graph (compute above the midline, bandwidth below). Token generation
  is bandwidth-bound once the model is resident, so the divergence between the
  two lines over time is the diagnosis: bandwidth pinned while compute idles
  means memory-bound, and a faster GPU core will not help. (#32)

- **Alert actions** — `alert_cmd` (or `--alert-cmd`) runs a shell command on
  every alert fire *and* resolve, with `TOPTOP_ALERT_STATE`,
  `TOPTOP_ALERT_SEVERITY`, `TOPTOP_ALERT_KEY`, `TOPTOP_ALERT_DETAIL` and
  `TOPTOP_ALERT_MSG` in its environment. Detached, output discarded, so a slow
  hook can't stall a tick or corrupt the TUI. (#11)
- **Built-in alert sinks** — `alert_webhook` (JSON), `alert_ntfy` and
  `alert_slack` POST fire/resolve transitions without writing any curl
  scripting. Payload construction is pure and unit-tested; delivery shells out
  to `curl` because Slack and ntfy.sh are HTTPS-only and toptop carries no TLS
  stack. Works in `--serve-metrics` mode too. (#39)
- **Alert history timeline + flap suppression** — a new `AlertTracker` turns
  each tick's alert set into debounced fire/resolve transitions: an alert that
  re-fires within `alert_flap_secs` (default 60) of its last announcement is
  tracked but not re-announced, and its matching resolve is suppressed with
  it, so the timeline never shows a resolve without its fire. Press `A` for the
  timeline, with relative ages and a suppressed-flap count. (#40)

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
- **Inference SLO triad** — TTFT *and* TPOT (time-per-output-token, the
  inter-token latency users feel while a response streams) as p50/p95/p99,
  derived by parsing the runtimes' Prometheus histogram buckets with linear
  in-bucket interpolation, the same way `histogram_quantile` does. Shown per
  server in the AI view and exported as
  `toptop_inference_{ttft,tpot}_p{50,95,99}_ms`. A quantile landing in the
  open-ended `+Inf` bucket reports the largest finite bound rather than
  inventing a number. (#28)
- **Prefill vs decode as a first-class split** — the AI view now shows the
  phase mix (`prefill 34% · decode 66%`) and names the dominant phase. The two
  phases have different bottlenecks — prefill is compute-bound, decode is
  memory-bandwidth-bound — so the mix says which one you are paying for. (#29)
- **SGLang metrics** — running/queued requests, KV (token-budget) usage, model
  name, generation throughput and TTFT are now parsed from SGLang's Prometheus
  endpoint; SGLang was detected as a runtime but showed no live numbers. (#9)
- **`--llm-server <host:port>`** — scrape inference servers that
  auto-discovery can't see: remote boxes, and macOS/Windows where the
  `/proc`-based socket→PID mapping doesn't exist. Repeatable, also settable as
  `llm_servers` in the config, IPv6 in bracket form. Manual targets are
  labelled by address and get a longer scrape timeout than the localhost
  sweep. (#13)
- **Richer process detail** — the `Enter` overlay now also lists the selected
  process's **open files**, **network sockets** and **environment**, fetched
  lazily for just that PID while the overlay is open and dropped as soon as it
  closes. Each section is capped to its share of the panel and reports what it
  elided (`… 37 more`), so an elided list never looks like a complete one. The
  overlay now grows with the terminal instead of being a fixed 72×18 box.
  Environment comes from sysinfo (all platforms); open files and sockets are
  `/proc`-based and are simply empty elsewhere. (#44)
- **Record & replay** — `--record <file>` appends one JSON snapshot per tick
  (JSONL; the `--export json` document plus a `schema_version`), and
  `--replay <file>` plays it back through the normal TUI: space pauses, ←/→
  step frame by frame, and the footer becomes a transport bar showing the
  position. Every panel renders from the recording, so a GPU incident can be
  replayed on a machine with no GPU. Recordings keep 100 process rows per
  frame rather than the export's top 20, an unparseable line (a recording
  killed mid-write) is skipped and counted rather than fatal, a version
  mismatch is reported on stderr, and a failing write stops recording with a
  visible status instead of silently truncating. The header shows `● REC`
  while recording. (#12)

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
