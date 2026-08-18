#!/usr/bin/env bash
# agentprof installer
#
# Works both inside a checkout and piped from the web:
#   ./install.sh
#   curl -fsSL https://raw.githubusercontent.com/dautovri/agentprof/main/install.sh | bash
#
# When run outside a checkout it clones the repository into a temporary
# directory first. The previous version ran `cargo build` in the caller's
# current directory, so the advertised curl|bash install always failed with
# "could not find Cargo.toml".

set -euo pipefail

REPO_URL="${AGENTPROF_REPO:-https://github.com/dautovri/agentprof.git}"
BOLD=''; CYAN=''; GREEN=''; YELLOW=''; RED=''; RESET=''
if [ -t 1 ]; then
  BOLD=$'\033[1m'; CYAN=$'\033[0;36m'; GREEN=$'\033[0;32m'
  YELLOW=$'\033[0;33m'; RED=$'\033[0;31m'; RESET=$'\033[0m'
fi

info()  { printf '%s\n' "${BOLD}$*${RESET}"; }
warn()  { printf '%s\n' "${YELLOW}$*${RESET}" >&2; }
die()   { printf '%s\n' "${RED}error:${RESET} $*" >&2; exit 1; }

CLONE_DIR=""
cleanup() { [ -n "$CLONE_DIR" ] && rm -rf "$CLONE_DIR"; }
trap cleanup EXIT

printf '%s\n\n' "${CYAN}${BOLD}⚡ Installing agentprof (AI Agent Workspace Optimizer)...${RESET}"

# 1. Rust toolchain
if ! command -v cargo >/dev/null 2>&1; then
  warn "Cargo/Rust not detected — installing via rustup."
  command -v curl >/dev/null 2>&1 || die "curl is required to install Rust."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
  # shellcheck disable=SC1091
  . "${CARGO_HOME:-$HOME/.cargo}/env"
fi
command -v cargo >/dev/null 2>&1 || die "cargo is still unavailable after installation."

# 2. Locate the source. Prefer an existing checkout; otherwise clone.
SCRIPT_DIR=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi

if [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/Cargo.toml" ]; then
  SRC_DIR="$SCRIPT_DIR"
  info "📁 Building from local checkout: $SRC_DIR"
else
  command -v git >/dev/null 2>&1 || die "git is required to install from the web."
  CLONE_DIR="$(mktemp -d)"
  info "📥 Cloning $REPO_URL ..."
  git clone --depth 1 "$REPO_URL" "$CLONE_DIR/agentprof" >/dev/null 2>&1 \
    || die "failed to clone $REPO_URL"
  SRC_DIR="$CLONE_DIR/agentprof"
fi

# 3. Build
info "🔨 Building optimized release binary..."
( cd "$SRC_DIR" && cargo build --release ) || die "build failed."

BINARY="$SRC_DIR/target/release/agentprof"
[ -x "$BINARY" ] || die "expected binary not found at $BINARY"

# 4. Choose a writable destination already on PATH where possible.
if [ -n "${AGENTPROF_INSTALL_DIR:-}" ]; then
  INSTALL_DIR="$AGENTPROF_INSTALL_DIR"
elif [ -w "/usr/local/bin" ]; then
  INSTALL_DIR="/usr/local/bin"
else
  INSTALL_DIR="${CARGO_HOME:-$HOME/.cargo}/bin"
fi
mkdir -p "$INSTALL_DIR"

info "📦 Installing to $INSTALL_DIR/agentprof ..."
install -m 755 "$BINARY" "$INSTALL_DIR/agentprof"

# 5. Verify, and warn if the destination is not on PATH.
printf '\n%s\n' "${GREEN}${BOLD}🎉 Installation successful!${RESET}"
"$INSTALL_DIR/agentprof" --version

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) warn "note: $INSTALL_DIR is not on your PATH. Add it with:
    export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac

printf '\n%s\n' "${BOLD}Quick start:${RESET}"
printf '  • Full scan:        %s\n' "${CYAN}agentprof scan${RESET}"
printf '  • Interactive TUI:  %s\n' "${CYAN}agentprof tui${RESET}"
printf '  • Measure MCP load: %s\n' "${CYAN}agentprof mcp --probe${RESET}"
printf '  • Preview fixes:    %s\n\n' "${CYAN}agentprof fix --dry-run${RESET}"
