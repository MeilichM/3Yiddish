#!/bin/sh
# 3Yiddish installer (Linux, macOS):
#   curl -fsSL https://github.com/TripleU613/3Yiddish/releases/latest/download/install.sh | sh
set -eu
REPO=TripleU613/3Yiddish
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) t=x86_64-unknown-linux-gnu ;;
  Linux-aarch64 | Linux-arm64) t=aarch64-unknown-linux-gnu ;;
  Darwin-x86_64) t=x86_64-apple-darwin ;;
  Darwin-arm64) t=aarch64-apple-darwin ;;
  *) echo "3Yiddish has no build for $(uname -s) $(uname -m) yet." >&2; exit 1 ;;
esac
base="https://github.com/$REPO/releases/latest/download"
dir="${THREE_YIDDISH_BIN:-$HOME/.local/bin}"
mkdir -p "$dir"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fetch() { if command -v curl >/dev/null 2>&1; then curl -fsSL "$1" -o "$2"; else wget -qO "$2" "$1"; fi; }
echo "Downloading 3Yiddish for $t…"
fetch "$base/3yiddish-$t" "$tmp/3yiddish"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS"
want=$(grep " 3yiddish-$t\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
got=$( (sha256sum "$tmp/3yiddish" 2>/dev/null || shasum -a 256 "$tmp/3yiddish") | cut -d' ' -f1)
[ -n "$want" ] && [ "$want" = "$got" ] || { echo "Checksum mismatch; nothing was installed." >&2; exit 1; }
chmod +x "$tmp/3yiddish"
mv "$tmp/3yiddish" "$dir/3yiddish"
echo "Installed: $dir/3yiddish"
case ":$PATH:" in *":$dir:"*) ;; *) echo "Add $dir to your PATH to run it from anywhere." ;; esac
"$dir/3yiddish" model pull || echo "Model download did not finish; run: 3yiddish model pull"
echo "Start it with: 3yiddish"
