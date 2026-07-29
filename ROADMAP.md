# toptop roadmap

## The one sentence

**toptop should be the one tool every local-LLM developer keeps open** — true on
Linux+NVIDIA *and* Apple Silicon, and deep enough to beat running nvtop + asitop
+ `tail -f vllm.log` side by side.

The moat is **unification**: GPU + inference server + system, auto-discovered,
zero-config, in one terminal. btop/glances don't understand serving; nvtop/nvitop
don't understand inference; DCGM + vLLM `/metrics` + Grafana does, but needs a
cluster. Everything below is judged by whether it widens that moat.

**The gap we're closing first:** the tagline says "local-inference observability
layer," but the AI view is dark on Apple Silicon — where the fastest-growing
local-LLM audience lives. Making the tagline true is the 1.1 focus.

## 1.1 — inference everywhere ([milestone](../../milestone/1))

Make the AI view true everywhere local LLMs run, and deepen it.

**Reach — make it work off Linux+NVIDIA:**
- [#4](../../issues/4) **Apple Silicon GPU** util + unified-memory pressure — the
  linchpin. (Precursor shipped: the AI view now states the platform honestly
  instead of claiming "No GPU detected".)
- [#13](../../issues/13) Manual `--llm-server host:port` — unblocks macOS and
  remote boxes for the inference half.
- [#5](../../issues/5) more runtimes · [#9](../../issues/9) SGLang ·
  [#10](../../issues/10) TGI parser — widen coverage (pure parsers, no hardware).

**Depth — make it worth keeping open:**
- [#28](../../issues/28) latency SLO triad (TTFT p50/p95/p99 + TPOT).
- [#30](../../issues/30) tokens/sec + KV-cache history sparklines.
- [#29](../../issues/29) prefill vs decode split.
- [#31](../../issues/31) KV-cache preemption / recompute counter.

### Open design decisions (resolve before #4 ships)
1. **Unified-memory metaphor.** Apple Silicon memory is unified — there's no
   separate VRAM to spill *from*; the risk is total memory pressure. Key the
   spill warning off Metal `recommendedMaxWorkingSetSize`, relabel "VRAM" as
   unified-memory headroom. (Tracked on [#4](../../issues/4).)
2. **Per-process GPU memory.** Metal/IOKit doesn't expose per-PID VRAM like NVML;
   the "GPU processes by VRAM" table may be dropped or re-sourced on macOS.
3. **macOS discovery parity.** `/proc` discovery is Linux-only; 1.1 is manual
   `--llm-server` ([#13](../../issues/13)), native macOS discovery is later.

## Deferred — breadth ([milestone](../../milestone/2))

Good work, deliberately not this cycle — it widens surface, not the moat:
[#33](../../issues/33)/[#35](../../issues/35) NVLink/PCIe + MIG ·
[#36](../../issues/36)/[#37](../../issues/37) cgroups + container/pod grouping ·
[#45](../../issues/45) Windows · [#42](../../issues/42) keybindings ·
[#43](../../issues/43) column customization.

## Hygiene (ongoing, not a milestone)

[#8](../../issues/8) packaging (crates.io/AUR/nixpkgs) ·
[#18](../../issues/18) CI hardening (macOS runner, MSRV, cargo-audit) ·
[#19](../../issues/19) unit tests for `app.rs`/`main.rs`.

## General backlog (unscheduled)

Export & integration ([#7](../../issues/7) Grafana, [#16](../../issues/16) CSV,
[#12](../../issues/12) record/replay, [#38](../../issues/38) OTLP), alerting
([#6](../../issues/6), [#11](../../issues/11), [#39](../../issues/39),
[#40](../../issues/40)), and quality-of-life
([#15](../../issues/15), [#17](../../issues/17), [#32](../../issues/32),
[#44](../../issues/44), [#46](../../issues/46)). Pulled into a milestone when it
serves the sentence at the top.

## Principle

toptop stays a **single, zero-dependency binary**. Anything needing a daemon,
database, or heavy runtime (long-term metric storage) is deferred to the export
path — Prometheus / OTLP / record-replay — not built in.
