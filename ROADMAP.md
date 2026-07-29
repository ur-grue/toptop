# toptop roadmap

## Where toptop sits

toptop is the only single-binary terminal tool that unifies **full system
monitoring** with **local-inference observability** — GPU compute‑vs‑memory‑
bandwidth, VRAM‑spill prediction, and auto‑discovered inference‑server metrics
(tokens/sec, KV cache, TTFT) — with zero runtime dependencies. General monitors
(btop, glances) don't understand the serving layer; GPU viewers (nvtop, nvitop)
don't understand inference; the production stack (DCGM + vLLM `/metrics` +
Grafana) does, but needs a cluster to stand up. toptop's north star is to be the
fastest way to answer *"is my GPU the bottleneck, and why?"* from one terminal —
then scale that same view to a fleet and out to Grafana.

The roadmap is organized by theme. Each theme lists **tracked** work (issues
already filed) and **proposed** work (new — surfaced by this roadmap, no issue
yet). The horizons at the end sequence it.

## Themes

### 1. Deeper inference telemetry — the differentiator

Tracked: [#5](../../issues/5) more runtimes · [#9](../../issues/9) SGLang ·
[#10](../../issues/10) TGI tokens/sec + TTFT · [#13](../../issues/13) manual
`--llm-server host:port`.

Proposed:
- **Latency SLO triad.** Alongside tokens/sec, surface **TTFT p50/p95/p99** and
  **TPOT** (time‑per‑output‑token / inter‑token latency) from the request‑level
  histograms vLLM/TGI already expose. TTFT, TPOT and throughput are the three
  metrics production teams alert on; today only mean throughput + a single TTFT
  are shown.
- **Prefill vs decode, first‑class.** The scraper already reads prefill rate —
  show prefill vs decode throughput side by side with a "which phase is the
  bottleneck" cue, mirroring the compute‑vs‑bandwidth framing.
- **Tokens/sec + KV‑cache history sparklines.** The collector already keeps
  256‑sample histories for CPU/mem/net; extend the same to tokens/sec and KV%
  so the AI view shows a trend, not just an instant.
- **KV‑cache preemption / recompute counter.** KV exhaustion surfaces as latency
  spikes; expose preemption/recompute events and queue‑backlog trend so a spill
  is visible before it becomes timeouts.

### 2. GPU depth

Tracked: [#4](../../issues/4) Apple Silicon GPU util + unified‑memory pressure.

Proposed:
- **GPU history graphs** in the AI view — compute % and memory‑bandwidth % over
  time, the signature comparison plotted rather than only barred.
- **Interconnect utilization** — NVLink / PCIe bandwidth. Multi‑GPU serving is
  often interconnect‑bound, and that's invisible today.
- **GPU‑aware process table** — a VRAM column (or GPU sort) in the main table so
  GPU‑heavy processes surface without switching to the AI view.
- **MIG / multi‑instance awareness** (longer term) — MIG slices as first‑class
  GPU rows.

### 3. Container & orchestration awareness

Tracked: none.

Proposed:
- **cgroup v2 limits.** Inside a container, key CPU/memory percentages off the
  cgroup quota, not host totals — a 2‑core container on a 64‑core host reads
  misleadingly today — and show usage‑vs‑limit.
- **Group by container / pod.** Label rows with the Docker container or
  Kubernetes pod name and offer a "group by container" view. Local inference
  increasingly runs in k8s; that's where the users are.

### 4. Export & integration breadth

Tracked: [#7](../../issues/7) ready‑made Grafana dashboard ·
[#16](../../issues/16) `--export csv` · [#12](../../issues/12) record & replay.

Proposed:
- **OpenTelemetry / OTLP export.** A push exporter to fit OTel pipelines (vLLM
  itself emits OTel), complementing the existing Prometheus pull endpoint.
- **Built‑in alert sinks** — webhook / ntfy / Slack as an option beyond "run a
  command" (extends [#11](../../issues/11)).

### 5. Alerting

Tracked: [#6](../../issues/6) configurable thresholds · [#11](../../issues/11)
alert actions (run a command).

Proposed:
- **Alert history & flap suppression.** A small in‑TUI timeline of recent
  fire/resolve transitions with debounce, so a flapping GPU doesn't spam actions
  (pairs with #11).

### 6. Interaction & process management

Tracked: [#17](../../issues/17) user‑defined themes · [#15](../../issues/15)
config polish.

Proposed:
- **renice / ioprio** — change a process's nice level (and I/O priority) from the
  same menu that already sends signals.
- **Configurable keybindings** via config file, matching the themes‑from‑config
  direction in #17.
- **Column customization** — choose / reorder / hide process‑table columns.
- **Richer process detail** — open files, environment, and per‑process network
  connections in the detail overlay.

### 7. Platform breadth

Tracked: [#4](../../issues/4) macOS GPU · [#8](../../issues/8) packaging
(crates.io, AUR, nixpkgs).

Proposed:
- **Windows support** (longer term). `sysinfo` already covers Windows for the
  system view; the AI view would use NVML. Rounds out the platform badge.

### 8. Project health

Tracked: [#18](../../issues/18) CI hardening (macOS runner, MSRV, cargo‑audit) ·
[#19](../../issues/19) unit tests for `app.rs` / `main.rs`.

Proposed:
- **Perf‑regression guard.** A small render/refresh benchmark in CI so the hot
  path — deliberately trimmed this cycle (throttled battery I/O, lazy exe/cwd) —
  doesn't regress silently.

## Horizons

**Now** — small, high‑leverage, mostly already filed:
[#10](../../issues/10) · [#9](../../issues/9) · [#16](../../issues/16) ·
[#6](../../issues/6) · [#15](../../issues/15) · [#7](../../issues/7); plus
tokens/sec history sparklines and the GPU‑aware VRAM column.

**Next** — the differentiator and reach:
[#4](../../issues/4) · [#13](../../issues/13) · [#11](../../issues/11) ·
[#12](../../issues/12) · [#5](../../issues/5); plus the latency SLO triad
(TTFT p95 / TPOT), prefill‑vs‑decode, cgroup limits, and OTLP export.

**Later** — bigger bets:
container/pod grouping; NVLink/PCIe + MIG; Windows support; configurable
keybindings and column customization.

## Notes

- **Issue [#14](../../issues/14)** ("signal menu reports 'signal' instead of
  SIGQUIT/SIGUSR1/SIGUSR2") is **resolved** by the merged signal‑menu work
  (#20) and can be closed.
- This roadmap keeps toptop a **single, zero‑dependency binary**. Anything that
  would need a daemon, database, or heavy runtime (e.g. long‑term metric
  storage) is deferred to the export path — Prometheus / OTLP / record‑replay —
  rather than built in.
