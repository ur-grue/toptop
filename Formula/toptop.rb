# Homebrew formula for toptop. The main repo doubles as its own tap:
#
#     brew tap ur-grue/toptop https://github.com/ur-grue/toptop
#     brew trust ur-grue/toptop     # one-time third-party-tap approval
#     brew install toptop
#
# The bleeding edge is also available with `brew install --HEAD toptop`.
class Toptop < Formula
  desc "Local-inference observability for the terminal (htop/btop-class monitor)"
  homepage "https://github.com/ur-grue/toptop"
  license "GPL-3.0-or-later"
  head "https://github.com/ur-grue/toptop.git", branch: "main"

  url "https://github.com/ur-grue/toptop/archive/refs/tags/v1.0.1.tar.gz"
  sha256 "a3ba36baaa84c25583f7787df9bd74ecabb4b78321364bedb12be58e18f10a36"

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
