class Arshy < Formula
  desc "AI Agent's native shell — structured output, security sandbox, MCP server"
  homepage "https://github.com/iZoy/Arshy"
  license "MIT"
  head "https://github.com/iZoy/Arshy.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "Arshy", shell_output("#{bin}/arshy --help")
  end
end
