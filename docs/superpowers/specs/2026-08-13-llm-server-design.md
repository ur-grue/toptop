# `--llm-server` — manual inference-server targets

**Issue:** [#13](https://github.com/ur-grue/toptop/issues/13)
**Date:** 2026-08-13
**Status:** Approved

## Problem

Inference-server auto-discovery walks `/proc` and is Linux-only. On macOS
(the fastest-growing local-LLM audience) and for remote GPU boxes in a
LAN the AI view shows "Localhost server discovery is Linux-only" — making
the tagline hollow on exactly the platform where it matters most.

## Solution

A new `--llm-server host:port` flag (and matching `llm_servers` config
key) lets users point toptop at any inference server — local or remote.
The existing Prometheus / Ollama / LM Studio parser chain handles the
rest; no new protocol work is needed.

## CLI & Config

### CLI

`--llm-server` accepts a comma-separated list or can be repeated:

```
toptop --llm-server gpu-box:8080,localhost:11434
toptop --llm-server gpu-box:8080 --llm-server localhost:11434
```

Port is mandatory — different runtimes use different ports, so guessing
a default would be wrong more often than right. Invalid entries are
rejected with a clear error message before the UI starts.

### Config file

`config.conf` gains a new key:

```
llm_servers = gpu-box:8080, localhost:11434
```

CLI overrides config (same semantics as every other flag). The key is
persisted on save when non-empty.

### Parsing

Each entry is split on the last `:` to extract `(host, port)`. The host
part may be an IP address or a hostname. The port is parsed as `u16`;
failure aborts with an error. A helper `parse_server_addr` in `config.rs`
handles both CLI and config-file parsing.

## Inference Monitor changes

### `InferenceMonitor::new(manual_targets)`

The constructor gains a `Vec<(String, u16)>` parameter. The background
thread stores these targets alongside the auto-discovery results.

### `http_get` generalization

Current signature: `fn http_get(port: u16, path: &str, timeout: Duration)`
— hardcodes `Ipv4Addr::LOCALHOST`.

New signature: `fn http_get(host: &str, port: u16, path: &str, timeout: Duration)`
— resolves `host` via `ToSocketAddrs` (works for IPs and hostnames). The
`Host` header is set to `host:port` for HTTP/1.1 correctness.

### `scrape_once` two-phase loop

1. **Auto-discovery** via `discover_servers()` — Linux-only, unchanged.
2. **Manual targets** — iterate `manual_targets`, call `http_get(host, port, …)`,
   run the same Prometheus → Ollama → LM Studio parser chain.

### Deduplication

A manual target that points to `localhost`/`127.0.0.1` on a port already
found by auto-discovery is skipped. Remote targets cannot collide with
auto-discovery by definition.

### PID handling

Auto-discovered servers have a real PID from `/proc`. Manual (especially
remote) servers have no PID — `pid: 0` serves as sentinel. The counter-
rate key type changes from `(u32, u16, &str)` to a new enum key that
covers both cases: `(pid, port, metric)` for local and
`(host_hash: u64, port, metric)` for remote, so rate tracking works for
both. A simple `hash(&host)` produces the `u64` discriminant.

### `ServerStats` changes

New field: `host: String`. Empty for auto-discovered localhost servers,
set to the configured hostname for manual targets. Used by the UI to
label remote servers and by the dedup/rate logic.

## UI changes (AI view)

### Server labels

Auto-discovered (host empty): `vLLM :8080` (unchanged).
Manual with host: `vLLM @ gpu-box:8080`.

### `no_servers_reason()`

The function gains a parameter `has_manual: bool`:

- Linux, no manual, no servers found: "No inference servers found on
  localhost." (unchanged)
- Non-Linux, no manual: "Localhost server discovery is Linux-only …"
  (unchanged, but the message no longer references issue #13 since the
  feature now exists)
- Manual configured but none responding: "Configured servers not
  responding."
- Manual configured and at least one responds: reason string is not
  shown (servers section is populated).

## Completions & man page

`--llm-server <HOST:PORT>` is added to:

- `completions/toptop.bash`
- `completions/_toptop` (zsh)
- `completions/toptop.fish`
- `man/toptop.1`
- The `--help` usage text in `main.rs`

## Testing

1. **Unit tests for `parse_server_addr`** — valid entries, missing port,
   invalid port, empty string, IPv6 `[::1]:8080`.
2. **Unit test for config round-trip** — `llm_servers` survives
   save → load.
3. **Unit test for dedup logic** — manual localhost target with same port
   as auto-discovered server is skipped.
4. **Existing parser tests** — unchanged, they already cover all runtime
   formats.
5. **Integration** — the render test suite exercises the AI view; adding
   a `ServerStats` with a non-empty `host` field covers the label path.

## Out of scope

- TLS / HTTPS for remote endpoints (inference servers expose plain HTTP
  metrics; TLS can be added later behind a `--llm-server-tls` flag if
  needed).
- Authentication (no inference server metrics endpoint requires auth
  today).
- DNS caching or connection pooling (the 2-second poll interval makes
  this unnecessary).
