#!/bin/sh
# Installs the crewai standalone binary on macOS and Linux.
#
#   curl -fsSL https://raw.githubusercontent.com/Hemant9785/crewAI/main/tools/install-crewai.sh | sh
#
# Optional environment variables:
#   CREWAI_VERSION      release tag to install, for example crewai-v0.1.0 (default: newest crewai-v* release)
#   CREWAI_INSTALL_DIR  where to put crewai (default: ~/.local/bin)
#   CREWAI_NO_PATH      set to 1 to leave your shell startup files unchanged
#   CREWAI_BASE_URL     folder URL holding the binary and SHA256SUMS (testing or mirrors)
#   CREWAI_OS, CREWAI_ARCH   override detection (testing): linux|macos, x64|arm64
set -eu

REPO="Hemant9785/crewAI"

say() { printf '%s\n' "$*"; }
fail() { printf 'crewai install: %s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "$1 is required but was not found."; }

need curl
need uname

os="${CREWAI_OS:-}"
if [ -z "$os" ]; then
  case "$(uname -s)" in
    Linux) os="linux" ;;
    Darwin) os="macos" ;;
    *) fail "unsupported system $(uname -s). On Windows use the PowerShell installer." ;;
  esac
fi
arch="${CREWAI_ARCH:-}"
if [ -z "$arch" ]; then
  case "$(uname -m)" in
    x86_64 | amd64) arch="x64" ;;
    arm64 | aarch64) arch="arm64" ;;
    *) fail "unsupported CPU $(uname -m). crewai supports x64 and arm64." ;;
  esac
fi
file="crewai-$os-$arch"

base="${CREWAI_BASE_URL:-}"
if [ -z "$base" ]; then
  tag="${CREWAI_VERSION:-}"
  if [ -z "$tag" ]; then
    say "Looking up the latest crewai release..."
    tag=$(curl -fsSL -H "User-Agent: crewai-installer" "https://api.github.com/repos/$REPO/releases?per_page=30" |
      grep -o '"tag_name": *"crewai-v[^"]*"' | head -n 1 | sed 's/.*"\(crewai-v[^"]*\)"/\1/') || true
    [ -n "$tag" ] || fail "no crewai release was found at https://github.com/$REPO/releases yet."
  fi
  base="https://github.com/$REPO/releases/download/$tag"
fi
base="${base%/}"

tmp=$(mktemp -d 2>/dev/null || mktemp -d -t crewai)
trap 'rm -rf "$tmp"' EXIT INT TERM

say "Downloading $file ..."
curl -fsSL "$base/$file" -o "$tmp/$file" || fail "could not download $base/$file"
curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" || fail "could not download $base/SHA256SUMS"

expected=$(grep -E " \*?$file\$" "$tmp/SHA256SUMS" | head -n 1 | cut -d ' ' -f 1)
[ -n "$expected" ] || fail "SHA256SUMS has no entry for $file."
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$tmp/$file" | cut -d ' ' -f 1)
elif command -v shasum >/dev/null 2>&1; then
  actual=$(shasum -a 256 "$tmp/$file" | cut -d ' ' -f 1)
else
  fail "sha256sum or shasum is required to verify the download."
fi
[ "$expected" = "$actual" ] || fail "checksum mismatch for $file (expected $expected, got $actual). Nothing was installed."
say "Checksum verified."

dir="${CREWAI_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$dir" || fail "could not create $dir"
chmod +x "$tmp/$file"
mv -f "$tmp/$file" "$dir/crewai" || fail "could not write $dir/crewai. If crewai is running, close it and run this again."
# A file downloaded with curl is not quarantined, but clear the flag if it is present.
if [ "$os" = "macos" ]; then xattr -d com.apple.quarantine "$dir/crewai" 2>/dev/null || true; fi

case ":$PATH:" in
  *":$dir:"*) on_path=1 ;;
  *) on_path=0 ;;
esac
if [ "$on_path" = 0 ] && [ "${CREWAI_NO_PATH:-0}" != "1" ]; then
  line="export PATH=\"$dir:\$PATH\""
  for rc in "$HOME/.profile" "$HOME/.bashrc" "$HOME/.zshrc"; do
    # Only touch files that exist (plus .profile, so there is always one).
    if [ -f "$rc" ] || [ "$rc" = "$HOME/.profile" ]; then
      grep -qsF "$line" "$rc" 2>/dev/null || printf '\n# crewai\n%s\n' "$line" >> "$rc"
    fi
  done
  say "Added $dir to your PATH in your shell startup files. Open a new terminal to pick it up."
fi

version=$("$dir/crewai" --version 2>/dev/null || echo "installed")
say ""
say "crewai $version is installed at $dir/crewai"
say "Open a project folder and run: crewai"
say "To look around first: crewai --demo"
