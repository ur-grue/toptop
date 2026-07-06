# Launch kit

Ready-to-paste posts for launching toptop. Order matters: cut the release
first, then post where the audience lives. Each post links the repo — stars
follow traffic, traffic follows a specific claim people can verify in 60s.

## 0. Pre-flight checklist

- [ ] `git tag -a v1.0.0 && git push origin v1.0.0` (triggers the release
      workflow → tarball + .deb attached to the GitHub Release)
- [x] Homebrew: the main repo is its own tap (`Formula/toptop.rb`, pinned to
      v1.0.0) — `brew tap ur-grue/toptop https://github.com/ur-grue/toptop`.
      Optionally create `ur-grue/homebrew-tap` later for the shorter
      `brew install ur-grue/tap/toptop` form.
- [ ] Repo social-preview image (Settings → General → Social preview): upload
      a render of `assets/ai-demo.svg` — it's what shows when the link is shared
- [ ] Verify README renders correctly on github.com/ur-grue/toptop (banner,
      animated hero, screenshots)

## 1. Show HN (news.ycombinator.com/submit)

**Title:**
> Show HN: Toptop – htop for local LLMs (tokens/sec, VRAM spill, the metrics nvidia-smi hides)

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
> It's also a full htop/btop-style monitor underneath. Read-only, localhost
> scrapes only, no telemetry. Happy to answer anything about the socket→PID
> mapping or the nvidia-smi parsing.

## 2. r/LocalLLaMA

**Title:**
> I made an htop alternative that shows why your local model is slow (mem-bandwidth vs compute, VRAM spill warnings, live tok/s from Ollama/vLLM/llama.cpp)

**Body:** lead with the animated demo, then the same pitch as HN but warmer;
end with "it's free/GPL, single binary — would love feedback on what metrics
you'd want next." Cross-post to r/selfhosted and r/rust (r/rust angle: zero-dep
JSON parser + hand-rolled HTTP + ratatui, 65 tests, headless TUI testing).

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
- **awesome-lists PRs**: awesome-rust (Utilities), awesome-selfhosted,
  awesome-llm / awesome-local-llm lists, terminal-apps lists
- **crates.io**: `cargo publish` (metadata already set) → free discovery
- **Ollama / vLLM Discords** — share in #show-and-tell style channels, framed
  as "a debugging tool for you", not an ad

## Rules of engagement

No fake engagement of any kind — no bought stars, no vote rings, no sockpuppet
comments. Besides being against every platform's ToS, this audience detects it
instantly and it torches credibility. The pitch is strong enough to stand on
the "verify it in 60 seconds" claim.
