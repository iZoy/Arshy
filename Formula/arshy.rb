class Arshy < Formula
  desc "AI Agent's structured execution layer — structured output, security sandbox, MCP server"
  homepage "https://github.com/iZoy/Arshy"
  license "MIT"
  head "https://github.com/iZoy/Arshy.git", branch: "main"
  # NOTE: this formula builds from source (requires the Rust toolchain).
  # After v0.1.0 artifacts are published, switch to prebuilt tarballs
  # (arshy-v<tag>-<rust-target>.tar.gz + sha256) for a seconds-fast install:
  #   on_macos do
  #     if Hardware::CPU.arm? then url ".../arshy-v0.1.0-aarch64-apple-darwin.tar.gz", sha256: "..."
  #     else url ".../arshy-v0.1.0-x86_64-apple-darwin.tar.gz", sha256: "..." end
  #   end
  #   on_linux do ... x86_64-unknown-linux-gnu / aarch64-unknown-linux-gnu ... end


  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "Arshy", shell_output("#{bin}/arshy --help")
  end
end
