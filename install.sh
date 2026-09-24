#!/usr/bin/env bash
# agentprof installer
#
#   curl -fsSL https://raw.githubusercontent.com/dautovri/agentprof/main/install.sh | bash
#   ./install.sh --from-source          # build this checkout with cargo
#
# Downloads the prebuilt binary for this platform from GitHub Releases and
# verifies its SHA-256 checksum before installing. When no binary exists for
# the platform (or with --from-source) it builds with cargo instead. It never
# installs a Rust toolchain on your behalf.
#
# Environment:
#   AGENTPROF_VERSION      release tag to install, e.g. v0.3.0 (default: latest)
#   AGENTPROF_INSTALL_DIR  destination directory (default: /usr/local/bin when
#                          writable, otherwise ~/.local/bin)
#   AGENTPROF_REPO         GitHub owner/repo to install from (default: dautovri/agentprof)

set -euo pipefail

REPO="${AGENTPROF_REPO:-dautovri/agentprof}"
VERSION="${AGENTPROF_VERSION:-latest}"
FROM_SOURCE=0

BOLD=''; CYAN=''; GREEN=''; YELLOW=''; RED=''; RESET=''
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  BOLD=$'\033[1m'; CYAN=$'\033[0;36m'; GREEN=$'\033[0;32m'
  YELLOW=$'\033[0;33m'; RED=$'\033[0;31m'; RESET=$'\033[0m'
fi

info()  { printf '%s\n' "${BOLD}$*${RESET}"; }
warn()  { printf '%s\n' "${YELLOW}$*${RESET}" >&2; }
die()   { printf '%s\n' "${RED}error:${RESET} $*" >&2; exit 1; }

usage() {
  sed -n '2,19p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//'
}

for arg in "$@"; do
  case "$arg" in
    --from-source) FROM_SOURCE=1 ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $arg (see --help)" ;;
  esac
done

TMP_DIR="$(mktemp -d)"
cleanup() { rm -rf "$TMP_DIR"; }
trap cleanup EXIT

printf '%s\n\n' "${CYAN}${BOLD}⚡ Installing agentprof...${RESET}"

# --- Where to install -------------------------------------------------------
if [ -n "${AGENTPROF_INSTALL_DIR:-}" ]; then
  INSTALL_DIR="$AGENTPROF_INSTALL_DIR"
elif [ -w "/usr/local/bin" ]; then
  INSTALL_DIR="/usr/local/bin"
else
  INSTALL_DIR="$HOME/.local/bin"
fi

# --- Platform ---------------------------------------------------------------
platform_asset() {
  local os arch
  case "$(uname -s)" in
    Darwin) os="darwin" ;;
    Linux) os="linux" ;;
    *) return 1 ;;
  esac
  case "$(uname -m)" in
    x86_64|amd64) arch="x86_64" ;;
    arm64|aarch64) arch="arm64" ;;
    *) return 1 ;;
  esac
  printf 'agentprof-%s-%s.tar.gz' "$os" "$arch"
}

fetch() {
  # fetch URL DEST
  if command -v curl >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -q "$1" -O "$2"
  else
    die "curl or wget is required to download agentprof."
  fi
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    die "sha256sum or shasum is required to verify the download."
  fi
}

# --- Prebuilt binary --------------------------------------------------------
install_binary() {
  local asset base expected actual
  asset="$(platform_asset)" || return 1
  if [ "$VERSION" = "latest" ]; then
    base="https://github.com/$REPO/releases/latest/download"
  else
    base="https://github.com/$REPO/releases/download/$VERSION"
  fi

  info "📥 Downloading $asset ($VERSION)..."
  fetch "$base/$asset" "$TMP_DIR/$asset" || return 1
  fetch "$base/$asset.sha256" "$TMP_DIR/$asset.sha256" \
    || die "no checksum published for $asset; refusing to install an unverified binary."

  expected="$(awk '{print $1}' "$TMP_DIR/$asset.sha256")"
  actual="$(sha256_of "$TMP_DIR/$asset")"
  [ -n "$expected" ] && [ "$expected" = "$actual" ] \
    || die "checksum mismatch for $asset (expected $expected, got $actual)."
  info "🔒 Checksum verified."

  tar -xzf "$TMP_DIR/$asset" -C "$TMP_DIR"
  [ -f "$TMP_DIR/agentprof" ] || die "archive did not contain an agentprof binary."
  BINARY="$TMP_DIR/agentprof"
}

# --- Build from source ------------------------------------------------------
install_from_source() {
  command -v cargo >/dev/null 2>&1 \
    || die "building from source needs Rust. Install it from https://rustup.rs and re-run."

  local src=""
  local script_dir=""
  if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  fi
  if [ -n "$script_dir" ] && [ -f "$script_dir/Cargo.toml" ]; then
    src="$script_dir"
    info "📁 Building from local checkout: $src"
  else
    command -v git >/dev/null 2>&1 || die "git is required to build from source."
    local ref=()
    [ "$VERSION" != "latest" ] && ref=(--branch "$VERSION")
    info "📥 Cloning https://github.com/$REPO ..."
    # ${ref[@]+...} keeps an empty array from tripping `set -u` on bash 3.2 (macOS).
    git clone --depth 1 ${ref[@]+"${ref[@]}"} "https://github.com/$REPO.git" "$TMP_DIR/src" >/dev/null 2>&1 \
      || die "failed to clone https://github.com/$REPO"
    src="$TMP_DIR/src"
  fi

  info "🔨 Building release binary (this takes a minute)..."
  ( cd "$src" && cargo build --release --locked ) || die "build failed."
  BINARY="$src/target/release/agentprof"
  [ -x "$BINARY" ] || die "expected binary not found at $BINARY"
}

BINARY=""
if [ "$FROM_SOURCE" = "1" ]; then
  install_from_source
elif ! install_binary; then
  warn "No prebuilt binary for $(uname -s)/$(uname -m) ($VERSION); building from source instead."
  install_from_source
fi

mkdir -p "$INSTALL_DIR"
info "📦 Installing to $INSTALL_DIR/agentprof ..."
install -m 755 "$BINARY" "$INSTALL_DIR/agentprof"

printf '\n%s\n' "${GREEN}${BOLD}🎉 Installed $("$INSTALL_DIR/agentprof" --version)${RESET}"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) warn "note: $INSTALL_DIR is not on your PATH. Add it with:
    export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

printf '\n%s\n' "${BOLD}Quick start:${RESET}"
printf '  • Full scan:          %s\n' "${CYAN}agentprof scan${RESET}"
printf '  • Protect secrets:    %s\n' "${CYAN}agentprof fix --dry-run${RESET}"
printf '  • Measure MCP load:   %s\n' "${CYAN}agentprof mcp --probe${RESET}"
printf '  • Interactive TUI:    %s\n\n' "${CYAN}agentprof tui${RESET}"
