# Homebrew formula for toptop.
#
# Intended to live in a tap (e.g. `ur-grue/homebrew-tap`) so users can:
#
#     brew install ur-grue/tap/toptop
#
# Until a tagged release tarball + sha256 are published you can install the tip
# of the default branch directly:
#
#     brew install --HEAD ur-grue/tap/toptop
#
# To cut a stable release, push a git tag (e.g. v1.0.0), then update `url` to the
# generated tarball and fill in `sha256` (shown by `brew fetch`/`shasum -a 256`).
class Toptop < Formula
  desc "Gorgeous, feature-rich terminal system monitor (htop/btop alternative)"
  homepage "https://github.com/ur-grue/toptop"
  license "GPL-3.0-or-later"
  head "https://github.com/ur-grue/toptop.git", branch: "main"

  # Stable release (uncomment and fill in once a tag is published):
  # url "https://github.com/ur-grue/toptop/archive/refs/tags/v1.0.0.tar.gz"
  # sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  version "1.0.0"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
    man1.install "man/toptop.1"
    bash_completion.install "completions/toptop.bash" => "toptop"
    zsh_completion.install "completions/_toptop"
    fish_completion.install "completions/toptop.fish"
  end

  test do
    assert_match "toptop #{version}", shell_output("#{bin}/toptop --version")
    # --snapshot is a non-interactive smoke test that needs no TTY.
    assert_match "snapshot", shell_output("#{bin}/toptop --snapshot")
  end
end
