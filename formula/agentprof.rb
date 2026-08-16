# typed: false
# frozen_string_literal: true

# Homebrew Formula for agentprof
# To install via tap:
#   brew tap dautovri/tap
#   brew install agentprof
#
# To install from local file:
#   brew install --build-from-source ./formula/agentprof.rb

class Agentprof < Formula
  desc "AI Agent Workspace Optimizer & Shell Startup Latency Profiler"
  homepage "https://github.com/dautovri/agentprof"
  url "https://github.com/dautovri/agentprof/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "022903fc97c37e6ca2daacfecad32992d8cf498fd7aba3af55dfbf51764fddd7"
  license "MIT"
  head "https://github.com/dautovri/agentprof.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args

    # Generate and install shell completions
    generate_completions_from_executable(bin/"agentprof", "completions")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/agentprof --version")
    output = shell_output("#{bin}/agentprof --help")
    assert_match "AI Agent Workspace Optimizer", output
  end
end
