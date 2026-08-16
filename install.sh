#!/usr/bin/env bash
# agentprof installer script
# Fast local build and installation to /usr/local/bin or ~/.cargo/bin

set -euo pipefail

BOLD='\033[1m'
CYAN='\033[0;36m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
RESET='\033[0m'

echo -e "${CYAN}${BOLD}⚡ Installing agentprof (AI Agent Workspace Optimizer)...${RESET}\n"

# 1. Check for Rust toolchain
if ! command -v cargo &> /dev/null; then
    echo -e "${YELLOW}Cargo/Rust is not detected. Installing via Homebrew or rustup...${RESET}"
    if command -v brew &> /dev/null; then
        brew install rust
    else
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
    fi
fi

# 2. Build release binary
echo -e "${BOLD}🔨 Building optimized release binary...${RESET}"
cargo build --release

# 3. Determine install destination
INSTALL_DIR="/usr/local/bin"
if [ ! -w "$INSTALL_DIR" ]; then
    INSTALL_DIR="$HOME/.cargo/bin"
    mkdir -p "$INSTALL_DIR"
fi

echo -e "${BOLD}📦 Copying binary to ${INSTALL_DIR}/agentprof...${RESET}"
cp "target/release/agentprof" "${INSTALL_DIR}/agentprof"
chmod +x "${INSTALL_DIR}/agentprof"

# 4. Verify installation
echo -e "\n${GREEN}${BOLD}🎉 Installation successful!${RESET}"
"${INSTALL_DIR}/agentprof" --version

echo -e "\n${BOLD}Quick Start:${RESET}"
echo -e "  • Full scan:       ${CYAN}agentprof scan${RESET}"
echo -e "  • Interactive TUI: ${CYAN}agentprof tui${RESET}"
echo -e "  • Profile MCP:     ${CYAN}agentprof mcp${RESET}"
echo -e "  • Profile Skills:  ${CYAN}agentprof skills${RESET}"
echo -e "  • Auto-fix:        ${CYAN}agentprof fix --all${RESET}\n"
