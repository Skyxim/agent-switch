class AgentSwitch < Formula
  desc "Fast, encrypted CLI switcher for AI agent credentials (Codex & Antigravity)"
  homepage "https://github.com/Skyxim/agent-switch"
  version "1.0.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/Skyxim/agent-switch/releases/download/v#{version}/ags-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "ac7b41c08ec87e7b58cfcd261c58f69d5ce95c8d57007fb219ea39250eea715e"
    end
    on_intel do
      url "https://github.com/Skyxim/agent-switch/releases/download/v#{version}/ags-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "7a0be1d8d1f0b3ebc47b74fbc9bc87a5f6f70ad957c1e90e52b6d6a64f30d100"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Skyxim/agent-switch/releases/download/v#{version}/ags-#{version}-aarch64-unknown-linux-musl.tar.gz"
      sha256 "76f94d7b1afb47dbddbf33b367a7f5755d6039ef46b748dbd6b9d0b7b5158e36"
    end
    on_intel do
      url "https://github.com/Skyxim/agent-switch/releases/download/v#{version}/ags-#{version}-x86_64-unknown-linux-musl.tar.gz"
      sha256 "726829dd2aa36813b021b5acecf128a21a79b20e349d8113e175d76470b9d8c5"
    end
  end

  def install
    bin.install "ags"
  end

  test do
    assert_match "ags", shell_output("#{bin}/ags --help")
  end
end
