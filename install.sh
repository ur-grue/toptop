#!/bin/sh
# toptop installer — fetches the release binary for this platform.
#
#   curl -fsSL https://raw.githubusercontent.com/ur-grue/toptop/main/install.sh | sh
#
# Options (as environment variables):
#   TOPTOP_VERSION=v1.2.3   install a specific release instead of the latest
#   TOPTOP_BIN_DIR=~/.local/bin   where to put the binary
#
# POSIX sh, no bashisms: this has to run under dash on a minimal container as
# readily as under zsh on a Mac.
set -eu

REPO="ur-grue/toptop"
BIN_DIR="${TOPTOP_BIN_DIR:-}"

die() {
    printf 'toptop install: %s\n' "$1" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "this needs '$1' on PATH"
}

# ── Which build do we want? ─────────────────────────────────────────────────
os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
    Linux) os_name="linux" ;;
    Darwin) os_name="macos" ;;
    MINGW* | MSYS* | CYGWIN*)
        die "on Windows, download the .zip from https://github.com/$REPO/releases/latest"
        ;;
    *) die "unsupported OS '$os' — build from source: https://github.com/$REPO#-install" ;;
esac
case "$arch" in
    x86_64 | amd64) arch_name="x86_64" ;;
    arm64 | aarch64) arch_name="aarch64" ;;
    *) die "unsupported architecture '$arch' — build from source: https://github.com/$REPO#-install" ;;
esac
platform="${arch_name}-${os_name}"

need uname
need tar
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1"; }
    fetch_to() { curl -fsSL -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO- "$1"; }
    fetch_to() { wget -qO "$2" "$1"; }
else
    die "this needs curl or wget on PATH"
fi

# ── Which version? ──────────────────────────────────────────────────────────
version="${TOPTOP_VERSION:-}"
if [ -z "$version" ]; then
    # Resolve the latest tag without needing jq: the redirect target of
    # /releases/latest ends in the tag.
    version="$(fetch "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1)"
    [ -n "$version" ] || die "could not determine the latest release — set TOPTOP_VERSION=vX.Y.Z"
fi
plain="${version#v}"

asset="toptop-${plain}-${platform}.tar.gz"
url="https://github.com/$REPO/releases/download/${version}/${asset}"

# ── Download, verify, install ───────────────────────────────────────────────
tmp="$(mktemp -d)"
# shellcheck disable=SC2064  # $tmp must expand now, not at trap time.
trap "rm -rf '$tmp'" EXIT INT TERM

printf 'toptop install: fetching %s (%s)\n' "$version" "$platform" >&2
fetch_to "$url" "$tmp/pkg.tar.gz" || die "no build for ${platform} in ${version}
  Available builds: https://github.com/$REPO/releases/${version}
  Or build from source: https://github.com/$REPO#-install"

tar -xzf "$tmp/pkg.tar.gz" -C "$tmp" || die "the download is not a valid archive"
bin="$(find "$tmp" -type f -name toptop -perm -u+x | head -n 1)"
[ -n "$bin" ] || die "no toptop binary inside ${asset}"

# Prove it runs here before putting it on the PATH: a wrong-architecture or
# too-old-glibc binary should fail now, with an explanation, not later.
chmod +x "$bin"
"$bin" --version >/dev/null 2>&1 || die "the downloaded binary does not run on this machine
  Your platform may need a source build: https://github.com/$REPO#-install"

# ── Where does it go? ───────────────────────────────────────────────────────
if [ -z "$BIN_DIR" ]; then
    # Prefer a user-writable directory already on PATH; never sudo silently.
    for candidate in "$HOME/.local/bin" "$HOME/bin" /usr/local/bin; do
        case ":$PATH:" in
            *":$candidate:"*)
                if [ -d "$candidate" ] && [ -w "$candidate" ]; then
                    BIN_DIR="$candidate"
                    break
                fi
                ;;
        esac
    done
fi
[ -n "$BIN_DIR" ] || BIN_DIR="$HOME/.local/bin"
mkdir -p "$BIN_DIR" || die "cannot create $BIN_DIR — set TOPTOP_BIN_DIR to somewhere writable"
[ -w "$BIN_DIR" ] || die "$BIN_DIR is not writable — set TOPTOP_BIN_DIR to somewhere else"

install -m 755 "$bin" "$BIN_DIR/toptop" 2>/dev/null \
    || { cp "$bin" "$BIN_DIR/toptop" && chmod 755 "$BIN_DIR/toptop"; } \
    || die "could not install into $BIN_DIR"

printf 'toptop install: installed %s to %s/toptop\n' "$version" "$BIN_DIR" >&2
case ":$PATH:" in
    *":$BIN_DIR:"*)
        printf '\nRun it:\n  toptop --demo\n' >&2
        ;;
    *)
        # shellcheck disable=SC2016  # $PATH is literal here: it is a line the
        # reader pastes into their shell, not something to expand now.
        printf '\n%s is not on your PATH. Add it:\n  export PATH="%s:$PATH"\n\nThen run:\n  toptop --demo\n' \
            "$BIN_DIR" "$BIN_DIR" >&2
        ;;
esac
