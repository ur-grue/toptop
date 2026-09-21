# Launch kit

Ready-to-paste posts for launching toptop. Order matters: cut the release
first, then post where the audience lives. Each post links the repo — stars
follow traffic, traffic follows a specific claim people can verify in 60s.

## 0. Pre-flight checklist

- [ ] **Tag v1.1.0** — the release that carries Apple Silicon and
      `--diagnose`. `git tag -a v1.1.0` (annotated) then `git push origin
      v1.1.0` triggers the release workflow (binaries for every platform +
      .deb on the GitHub Release). v1.0.0–v1.0.2 are done.
- [x] Homebrew: the main repo is its own tap (`Formula/toptop.rb`) — `brew tap
      ur-grue/toptop https://github.com/ur-grue/toptop`. Optionally create
      `ur-grue/homebrew-tap` later for the shorter `brew install
      ur-grue/tap/toptop` form.
      **Re-pin `url` + `sha256` on every release** — a stale formula installs
      an old version on the first wave of traffic. For v1.1.0:
      `curl -sL https://github.com/ur-grue/toptop/archive/refs/tags/v1.1.0.tar.gz | shasum -a 256`
- [ ] `cargo publish` after the tag (1.0.2 is on crates.io).
- [ ] Upload `assets/social-preview.png` as the repo social-preview image
      (Settings → General → Social preview). Without it every shared link is
      the grey default card. One minute; there is no API for it.
- [x] Homepage URL set, Discussions enabled (2026-09-21).
- [ ] Verify README renders correctly on github.com/ur-grue/toptop (banner,
      animated hero, the `--diagnose` card block)

## 1. Show HN (news.ycombinator.com/submit)

**Title:**
> Show HN: Toptop – htop for local LLMs, with a paste-able "why is it slow?" verdict

**URL:** `https://github.com/ur-grue/toptop`

**First comment (post immediately after submitting):**

> Author here. I built this because `nvidia-smi` kept telling me my GPU was at
> "30% utilization" while my local Llama was crawling — and the real answer was
> memory bandwidth, which nvidia-smi doesn't surface next to anything useful.
>
> toptop is a terminal monitor (Rust, ~1.3 MB, zero runtime deps) with a view
> built for local inference:
>
> - compute vs **memory-bandwidth** utilization side by side (token generation
>   is bandwidth-bound once the model is resident)
> - VRAM headroom with a warning **before** the model spills to system RAM
>   (that's the 5–20× slowdown everyone hits once)
> - it finds your inference servers by scanning listening sockets → PIDs, then
>   scrapes vLLM/llama.cpp/TGI `/metrics` and Ollama `/api/ps` for live
>   tokens/sec, KV-cache %, queue depth, TTFT — no config
> - throttle-reason flags, tokens/sec/watt, per-process VRAM
> - `--serve-metrics` = a Prometheus endpoint; `--remote host1,host2` = a
>   fleet view over plain SSH (no agents)
>
> New in 1.1: it runs on Apple Silicon (GPU util + unified-memory pressure
> via IOKit/Metal, no root, no extra crates — asitop has been unmaintained
> since 2024), and `toptop --diagnose` prints the verdict as a 72-column
> text card you can paste into an issue or a thread, numbers and advice
> included.
>
> No GPU? `toptop --demo` simulates a busy 4090 + vLLM server so you can see
> the whole AI view (spill warning included) on any machine.
>
> It's also a full htop/btop-style monitor underneath. Read-only, localhost
> scrapes only, no telemetry. Happy to answer anything about the socket→PID
> mapping or the nvidia-smi parsing.

## 2. r/LocalLLaMA

**Title:**
> I made an htop alternative that tells you *why* your local model is slow — and prints a verdict you can paste here (Ollama/vLLM/llama.cpp, NVIDIA + Apple Silicon)

**Body:** lead with this real `toptop --diagnose` card (Intel iMac, Ollama
running Mistral Small 24B entirely on the CPU — taken 2026-09-21, replace
with a fresher one if you have it), then the animated AI-view demo, then the same pitch as HN but warmer. Ask the
question that turns readers into users: "if your model is slow, run
`toptop --diagnose` and paste it — I'll read every one." End with "it's
free/GPL, single binary — what metric would you want next?"

Real card to open the post with:

```text
toptop diagnose  ·  v1.1.0  ·  2026-09-21 10:21 UTC
========================================================================
HOST     imac
VERDICT  MODEL RUNNING ON CPU
         Ollama:11434 · mistral-small3.1:24b 0% on GPU
         The runtime is not using the GPU at all — no supported backend
         for this card, or it was started CPU-only. Check `ollama ps`
         (PROCESSOR column) and the server log's GPU detection line
         before tuning anything else.

GPU      AMD Radeon Pro 5700 XT
         compute 9% · vram 3.8 GiB / 16.0 GiB (24%) · 61°C · 18 W
SERVER   Ollama:11434 mistral-small3.1:24b
         0% on GPU
SYSTEM   macOS 26.5.2 · x86_64 · 20c · cpu 63% · ram 82.8 GiB / 128 GiB
         (65%) · swap 1.7 GiB
------------------------------------------------------------------------
toptop v1.1.0 · https://github.com/ur-grue/toptop · `toptop --diagnose`
```

**Apple Silicon post (separate, a few days later, r/LocalLLaMA + r/macapps):**
> asitop is dead since 2024 — I built GPU util + unified-memory pressure +
> live Ollama tok/s for Apple Silicon into one htop-style monitor

Cross-post the main post to r/selfhosted and r/rust (r/rust angle: zero-dep
JSON parser + hand-rolled HTTP + raw IOKit/Metal FFI + ratatui, 219 tests,
headless TUI testing).

## 3. X/Twitter thread (attach a screen recording of the AI view)

1. Your GPU says 30% utilization. Your local Llama is crawling. Both are true —
   and nvidia-smi won't tell you why. 🧵
2. Token generation is memory-bandwidth-bound once the model fits in VRAM.
   toptop shows compute vs bandwidth side by side — the bottleneck is obvious
   in one glance.
3. It also auto-discovers Ollama / vLLM / llama.cpp servers and streams live
   tokens/sec, KV-cache pressure, and time-to-first-token into your terminal.
   Zero config.
4. VRAM spill is the silent killer: cross the limit and layers offload to RAM
   at 5–20× slower. toptop warns you *before* it happens — in the TUI and as a
   Prometheus alert.
5. Rust, 1.3 MB, zero deps, GPL. github.com/ur-grue/toptop

## 4. Slow-burn channels

- **This Week in Rust** — submit to the "Crate of the Week" thread
- **List PRs** — exact entries and order in `docs/LISTINGS.md`: Ollama
  community integrations (first: biggest audience, no star gate),
  awesome-tuis, awesome-ratatui; awesome-rust once >50 stars.
- **Answer threads, don't post ads**: every "why is my model slow" thread on
  r/LocalLLaMA, r/ollama and the Ollama/llama.cpp issue trackers gets a
  reply that is the actual diagnosis plus "here's the `toptop --diagnose`
  card from my box". The card carries the link.
- **crates.io**: `cargo publish` after each tag. "is it on crates.io?" is
  always the first comment.
- **AUR**: `packaging/aur/PKGBUILD`, steps in `docs/LISTINGS.md`. Arch users
  are disproportionately TUI users.
- **Ollama / vLLM Discords** — share in #show-and-tell style channels, framed
  as "a debugging tool for you", not an ad

## Rules of engagement

No fake engagement of any kind — no bought stars, no vote rings, no sockpuppet
comments. Besides being against every platform's ToS, this audience detects it
instantly and it torches credibility. The pitch is strong enough to stand on
the "verify it in 60 seconds" claim.
