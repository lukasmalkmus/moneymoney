# frozen_string_literal: true
#
# Template for a Homebrew formula to distribute `mm`. Lives in the
# companion tap repo (e.g. `lukasmalkmus/homebrew-tap`) at
# `Formula/mm.rb`. Keep this file as a reference only — Homebrew never
# reads it from this repo.
#
# Release-workflow appendix to automate bumps:
#
#   1. After `taiki-e/create-gh-release-action` publishes a release,
#      add a follow-up job that computes sha256 of each macOS archive.
#   2. Commit the updated formula (version + sha256) to the tap repo
#      via a GitHub App token with `contents: write` on that repo.
#   3. Open a PR or push directly to the tap's default branch.
#
# Sketch of the bump job (pseudocode — paste into release.yaml under a
# new `formula-bump` job `needs: [release]`):
#
#   - checkout lukasmalkmus/homebrew-tap using a TAP_GITHUB_APP_TOKEN
#   - for each of mm-aarch64-apple-darwin.tar.gz and mm-x86_64-apple-darwin.tar.gz:
#       sha=$(curl -sL <release-url>/$archive | shasum -a 256 | cut -d' ' -f1)
#       sed -i "s|sha256 \".*\" # arm64|sha256 \"$sha\" # arm64|" Formula/mm.rb
#     (or use a small Ruby / jq script)
#   - commit "mm 0.3.0" and push

class Mm < Formula
  desc "Agent-native CLI and MCP server for MoneyMoney"
  homepage "https://github.com/lukasmalkmus/moneymoney"
  version "0.3.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/lukasmalkmus/moneymoney/releases/download/v#{version}/mm-aarch64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_ARM64_SHA256"
    end
    on_intel do
      url "https://github.com/lukasmalkmus/moneymoney/releases/download/v#{version}/mm-x86_64-apple-darwin.tar.gz"
      sha256 "REPLACE_WITH_X86_64_SHA256"
    end
  end

  def install
    bin.install "mm"
  end

  test do
    assert_match "mm #{version}", shell_output("#{bin}/mm --version")
  end
end
