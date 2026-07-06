# Contributing to toptop

Thanks for your interest! toptop aims to be the local-inference observability
layer for the terminal, and contributions of every size are welcome.

## Quick start

```bash
cargo test                          # unit + headless render tests (no TTY needed)
cargo clippy --all-targets -- -D warnings
cargo fmt --all
cargo run --example preview 120 40  # render one frame as plain text
cargo run -- --snapshot             # non-interactive smoke test
```

CI runs exactly those checks (`.github/workflows/ci.yml`), so a green local
run means a green PR.

## Ground rules

- **Zero runtime dependencies is a feature.** The JSON parser, HTTP client and
  Prometheus serializer are hand-rolled on purpose. New functionality should
  not add crates unless there's a very strong case.
- **Pure core, thin I/O.** Parsers and rule engines (`metrics::gpu`,
  `metrics::infer`, `metrics::ai`, `json`, `alerts`, `fleet::parse_host_json`)
  are pure functions with unit tests; anything touching `/proc`, sysfs, sockets
  or subprocesses stays in a thin wrapper that degrades gracefully.
- **The UI must never block.** Slow sources (nvidia-smi, HTTP scrapes, SSH)
  run on background threads that publish into shared state.
- **Every view renders at every size.** New UI goes through the headless
  render tests in `tests/render.rs`, from 1×1 upward.

## High-value areas

- More inference runtimes (MLC, TensorRT-LLM, llamafile metrics, …)
- AMD GPU depth (utilization on more kernels), Apple Silicon support
- Fleet view drill-down; alert threshold configuration
- Packaging: AUR, nixpkgs, Homebrew core

## Reporting bugs

`toptop --export json` output (redact hostnames if you like) plus your
terminal + GPU model makes almost any report actionable.
