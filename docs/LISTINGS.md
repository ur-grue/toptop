# Listings — permanent referral traffic

Each entry below is a one-line PR to a list people actually browse. Every one
is a durable link; none needs a star count except awesome-rust (>50 stars or
>2000 crates.io downloads — submit after the HN/Reddit wave).

Submit in this order. Each takes ~3 minutes via the GitHub "edit" pencil.

## 1. Ollama — Community Integrations → Terminal & CLI

File: `README.md` in https://github.com/ollama/ollama, section `### Terminal & CLI`.
Append at the end of that list:

```markdown
- [toptop](https://github.com/ur-grue/toptop) - Terminal monitor for local inference: live tokens/sec, KV-cache and GPU-offload from `/api/ps`, VRAM-spill and bandwidth-bound verdicts
```

PR title: `docs: add toptop to Terminal & CLI integrations`

PR body:
> toptop is a Rust terminal monitor that auto-discovers a running Ollama
> server, reads `/api/ps`, and shows live tokens/sec, how much of the model is
> on the GPU, and a verdict when a model did not fit ("MODEL PARTLY ON CPU").
> Single binary, no runtime deps, GPL-3.0. `toptop --demo` shows the view
> without a GPU.

## 2. awesome-tuis — https://github.com/rothgar/awesome-tuis

File: `README.md`, section **System Monitoring** (alphabetical; goes after
`tmd-top`):

```markdown
- [toptop](https://github.com/ur-grue/toptop) htop/btop-class system monitor with a local-LLM view: tokens/sec, VRAM-spill and memory-bandwidth verdicts, Apple Silicon + NVIDIA
```

PR title: `Add toptop`

## 3. awesome-ratatui — https://github.com/ratatui/awesome-ratatui

File: `README.md`, section **Monitoring, Diagnostics, and Logs** (alphabetical;
after `rrtop`, before `tuistash`):

```markdown
- [toptop](https://github.com/ur-grue/toptop) - System monitor with a local-inference view: live tokens/sec, GPU compute-vs-bandwidth, VRAM-spill verdicts.
```

PR title: `Add toptop`

## 4. ratatui.rs showcase (optional, after a GIF exists)

https://github.com/ratatui/ratatui-website — `src/content/docs/showcase/apps/`
wants a screenshot or GIF. Render `assets/ai-demo.svg` to a GIF first.

## 5. AUR — after v1.1.0 is tagged

`packaging/aur/PKGBUILD` in this repo is ready. Needs an AUR account + SSH key:

```sh
git clone ssh://aur@aur.archlinux.org/toptop.git aur-toptop
cp packaging/aur/PKGBUILD aur-toptop/ && cd aur-toptop
updpkgsums && makepkg --printsrcinfo > .SRCINFO
git add PKGBUILD .SRCINFO
git commit -m "chore: release 1.1.0"
git push
```

## 6. awesome-rust — once `stars > 50`

https://github.com/rust-unofficial/awesome-rust, section **System tools**,
template from their CONTRIBUTING:

```markdown
* [ur-grue/toptop](https://github.com/ur-grue/toptop) [[toptop](https://crates.io/crates/toptop)] - htop-class system monitor with a local-LLM view: tokens/sec, VRAM-spill and bandwidth-bound verdicts [![CI](https://github.com/ur-grue/toptop/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/ur-grue/toptop/actions?query=branch%3Amain)
```
