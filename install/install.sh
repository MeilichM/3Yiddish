#!/bin/sh
# 3Yiddish installer (Linux, macOS, Android/Termux):
#   curl -fsSL https://github.com/TripleU613/3Yiddish/releases/latest/download/install.sh | sh
# Linux x86_64 has two builds: one that uses the graphics card (Vulkan) and a regular one.
# The installer picks; THREE_YIDDISH_VARIANT=cpu or =gpu forces the choice.
set -eu
REPO=TripleU613/3Yiddish
os=$(uname -s)
# Termux on an Android phone is not regular Linux underneath; it gets its own build.
if [ "$(uname -o 2>/dev/null)" = Android ] || [ -n "${TERMUX_VERSION:-}" ]; then os=Android; fi
case "$os-$(uname -m)" in
  Android-aarch64 | Android-arm64) t=aarch64-linux-android ;;
  Linux-x86_64) t=x86_64-unknown-linux-gnu ;;
  Linux-aarch64 | Linux-arm64) t=aarch64-unknown-linux-gnu ;;
  Darwin-x86_64) t=x86_64-apple-darwin ;;
  Darwin-arm64) t=aarch64-apple-darwin ;;
  *) echo "3Yiddish has no build for $(uname -s) $(uname -m) yet." >&2; exit 1 ;;
esac
base="${THREE_YIDDISH_BASE:-https://github.com/$REPO/releases/latest/download}"
if [ "$os" = Android ] && [ -n "${PREFIX:-}" ]; then
  dir="${THREE_YIDDISH_BIN:-$PREFIX/bin}"
else
  dir="${THREE_YIDDISH_BIN:-$HOME/.local/bin}"
fi
mkdir -p "$dir"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fetch() { if command -v curl >/dev/null 2>&1; then curl -fsSL "$1" -o "$2"; else wget -qO "$2" "$1"; fi; }

# The graphics-card build needs the system's Vulkan library (it comes with the graphics driver).
has_vulkan() {
  for l in ldconfig /sbin/ldconfig /usr/sbin/ldconfig; do
    if "$l" -p 2>/dev/null | grep -q 'libvulkan\.so\.1'; then return 0; fi
  done
  for d in /usr/lib/x86_64-linux-gnu /usr/lib64 /usr/lib /lib/x86_64-linux-gnu /lib64 /usr/local/lib; do
    if [ -e "$d/libvulkan.so.1" ]; then return 0; fi
  done
  return 1
}

# get NAME: download one build, check it against SHA256SUMS and make sure it runs here.
# Leaves it at $tmp/3yiddish. On failure sets $why and returns 1.
get() {
  rm -f "$tmp/3yiddish"
  why="The download failed"
  fetch "$base/$1" "$tmp/3yiddish" || return 1
  [ -s "$tmp/SHA256SUMS" ] || fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || return 1
  want=$(grep "[ *]$1\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)
  got=$( (sha256sum "$tmp/3yiddish" 2>/dev/null || shasum -a 256 "$tmp/3yiddish") | cut -d' ' -f1)
  why="Checksum mismatch"
  [ -n "$want" ] && [ "$want" = "$got" ] || return 1
  chmod +x "$tmp/3yiddish"
  why="This build does not run on this system ($t)"
  "$tmp/3yiddish" --version >/dev/null 2>&1 || return 1
}

gpu=no
if [ "$t" = x86_64-unknown-linux-gnu ]; then
  case "${THREE_YIDDISH_VARIANT:-auto}" in
    gpu | vulkan) gpu=yes ;;
    cpu) ;;
    auto | "")
      if has_vulkan; then
        gpu=yes; echo "Using the graphics card (GPU) build."
      else
        echo "No graphics driver found; using the regular build."
      fi ;;
    *) echo "THREE_YIDDISH_VARIANT must be cpu or gpu." >&2; exit 1 ;;
  esac
fi
echo "Downloading 3Yiddish for $t…"
ok=no
if [ "$gpu" = yes ]; then
  if get "3yiddish-$t-vulkan"; then
    ok=yes
  else
    echo "The graphics card (GPU) build did not work here; using the regular build."
  fi
fi
if [ "$ok" = no ]; then
  get "3yiddish-$t" || { echo "$why; nothing was installed." >&2; exit 1; }
fi
mv "$tmp/3yiddish" "$dir/3yiddish"
echo "Installed: $dir/3yiddish"
case ":$PATH:" in *":$dir:"*) ;; *) echo "Add $dir to your PATH to run it from anywhere." ;; esac
[ -z "${THREE_YIDDISH_SKIP_MODEL:-}" ] || exit 0
"$dir/3yiddish" model pull || echo "Model download did not finish; run: 3yiddish model pull"
if [ "$os" = Android ]; then
  echo "For the microphone, clipboard and playback: install the Termux:API app (F-Droid), then"
  echo "  pkg install termux-api play-audio"
fi
echo "Start it with: 3yiddish"
