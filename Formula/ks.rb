class Ks < Formula
  desc "Command-line client for Koroai Supply"
  homepage "https://github.com/koroai-supply/cli"
  url "https://github.com/koroai-supply/cli/archive/ce129a91b30f6a10d64d350a3e5a396767c82010.tar.gz"
  version "0.1.0"
  sha256 "8afdbb030a2b73f2d442aee81451ba4a4ec4d3949ccdf22001763bcd40061ea3"
  license "MIT"

  depends_on "rust" => :build

  deny_network_access!

  def fetch
    system "cargo", "fetch", *std_cargo_fetch_args
  end

  def install
    system "cargo", "install", *std_cargo_args
  end

  test do
    assert_match "頃合商店 CLI", shell_output("#{bin}/ks --help")
    assert_match version.to_s, shell_output("#{bin}/ks --version")
  end
end
