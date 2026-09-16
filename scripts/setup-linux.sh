#!/usr/bin/env bash
# JupiterOS / Tool-AI — Linux setup script.
#
# Provisions a development daily-driver on a fresh Debian/Ubuntu box:
#  - system deps (Tauri GUI native libs, build tools, Python venv, Node)
#  - Rust workspace (Rust Moons + shared) in release
#  - Tauri GUI (app) in release
#  - Python venv for Moon Amalthea
#  - .mcp.json bootstrapped from .mcp.json.example with Linux binary paths
#
# Qdrant binary + ONNX BGE-M3 model are NOT installed by this script:
# moon-io's setup.rs auto-downloads them on first run (~660 MB combined).
#
# Credentials (IMAP password, Gmail OAuth, Telegram session) are NOT touched.
# After this script finishes, run:
#   ./target/release/moon-io credentials set aruba
#   ./target/release/moon-io oauth connect gmail
# to populate the system keyring (gnome-keyring / kwallet via secret-service).

set -euo pipefail

REPO_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_DIR"

log()  { printf '\033[1;34m[setup]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[warn]\033[0m  %s\n' "$*" >&2; }
die()  { printf '\033[1;31m[fail]\033[0m  %s\n' "$*" >&2; exit 1; }

# ---------------------------------------------------------------------------
# 1. System dependencies
# ---------------------------------------------------------------------------
install_system_deps() {
    log "Installing system dependencies (sudo apt)…"
    if ! command -v apt-get >/dev/null 2>&1; then
        die "This script supports Debian/Ubuntu (apt-get) only. Adapt it for your distro."
    fi

    sudo apt-get update
    sudo apt-get install -y \
        build-essential \
        curl \
        git \
        pkg-config \
        libssl-dev \
        libwebkit2gtk-4.1-dev \
        libsoup-3.0-dev \
        libjavascriptcoregtk-4.1-dev \
        libgtk-3-dev \
        libayatana-appindicator3-dev \
        librsvg2-dev \
        libdbus-1-dev \
        libsecret-1-dev \
        python3-venv \
        python3-pip

    # Node.js (Tauri GUI build needs npm). Skip if a recent node is already there.
    if ! command -v node >/dev/null 2>&1 || [[ "$(node -v | sed 's/v//; s/\..*//')" -lt 20 ]]; then
        log "Installing Node.js 20.x via NodeSource…"
        curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
        sudo apt-get install -y nodejs
    fi

    # Rust toolchain via rustup (project pins via rust-toolchain.toml).
    if ! command -v cargo >/dev/null 2>&1; then
        log "Installing Rust toolchain via rustup…"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
        # shellcheck disable=SC1091
        source "$HOME/.cargo/env"
    fi

    # Tauri CLI (npm package, project-local — cargo plugin not required).
    log "System deps OK."
}

# ---------------------------------------------------------------------------
# 2. Rust workspace (Rust Moons + shared) — release build
# ---------------------------------------------------------------------------
build_rust_workspace() {
    log "Building Rust workspace (release)…"
    cargo build --release --workspace
    log "Rust binaries: $REPO_DIR/target/release/{moon-io,moon-europa}"
}

# ---------------------------------------------------------------------------
# 3. Tauri GUI (app)
# ---------------------------------------------------------------------------
build_tauri_gui() {
    log "Installing JS deps for Tauri GUI…"
    pushd app >/dev/null
    npm install
    log "Building Tauri GUI (release) — this can take 3-5 minutes…"
    npm run tauri build
    popd >/dev/null
    log "GUI binary: $REPO_DIR/app/src-tauri/target/release/jupiteros"
}

# ---------------------------------------------------------------------------
# 4. Moon Amalthea — Python venv
# ---------------------------------------------------------------------------
setup_amalthea_venv() {
    log "Setting up Moon Amalthea Python venv…"
    pushd moons/amalthea >/dev/null
    python3 -m venv .venv
    # shellcheck disable=SC1091
    source .venv/bin/activate
    pip install --upgrade pip
    pip install -e .
    deactivate
    popd >/dev/null
    log "Amalthea venv ready: $REPO_DIR/moons/amalthea/.venv"
}

# ---------------------------------------------------------------------------
# 4b. Moon Google Ads — Python venv (Keyword Planner + GAQL reporting)
# ---------------------------------------------------------------------------
setup_google_ads_venv() {
    log "Setting up Moon Google Ads Python venv…"
    pushd moons/google-ads >/dev/null
    python3 -m venv .venv
    # shellcheck disable=SC1091
    source .venv/bin/activate
    pip install --upgrade pip
    pip install -e .
    deactivate
    popd >/dev/null
    log "Google Ads venv ready: $REPO_DIR/moons/google-ads/.venv"
}

# ---------------------------------------------------------------------------
# 4c. Moon Meta Ads — Python venv (insights, interest research, ad set creation)
# ---------------------------------------------------------------------------
setup_meta_ads_venv() {
    log "Setting up Moon Meta Ads Python venv…"
    pushd moons/meta-ads >/dev/null
    python3 -m venv .venv
    # shellcheck disable=SC1091
    source .venv/bin/activate
    pip install --upgrade pip
    pip install -e .
    deactivate
    popd >/dev/null
    log "Meta Ads venv ready: $REPO_DIR/moons/meta-ads/.venv"
}

# ---------------------------------------------------------------------------
# 5. .mcp.json — bootstrap from example with Linux binary paths
# ---------------------------------------------------------------------------
bootstrap_mcp_json() {
    if [[ -f .mcp.json ]]; then
        warn ".mcp.json already exists — leaving it untouched."
        warn "  Edit it manually if binary paths are stale."
        return
    fi
    if [[ ! -f .mcp.json.example ]]; then
        warn "No .mcp.json.example found — skipping config bootstrap."
        return
    fi

    log "Creating .mcp.json from example with Linux paths…"
    # Windows → Linux: drop .exe and swap venv path.
    sed \
        -e 's|target/release/moon-io\.exe|target/release/moon-io|g' \
        -e 's|target/release/moon-europa\.exe|target/release/moon-europa|g' \
        -e 's|target/release/moon-ganymede\.exe|target/release/moon-ganymede|g' \
        -e 's|target/release/moon-metis\.exe|target/release/moon-metis|g' \
        -e 's|target/release/moon-himalia\.exe|target/release/moon-himalia|g' \
        -e 's|target/release/moon-elara\.exe|target/release/moon-elara|g' \
        -e 's|\.venv/Scripts/python\.exe|.venv/bin/python|g' \
        .mcp.json.example > .mcp.json
    log ".mcp.json created — review and fill in your credentials/env vars."
}

# ---------------------------------------------------------------------------
# 6. Final hints
# ---------------------------------------------------------------------------
print_next_steps() {
    cat <<EOF

\033[1;32m✓ Setup complete.\033[0m

Next steps:

1. Review and edit .mcp.json (account email, Telegram credentials, etc.).

2. Populate the system keyring with your credentials:
     ./target/release/moon-io credentials set aruba
     ./target/release/moon-io oauth connect gmail   # opens browser

3. Launch the JupiterOS GUI:
     ./app/src-tauri/target/release/jupiteros

On first launch:
  - Moon Io's setup.rs auto-downloads Qdrant (~80 MB) and the BGE-M3 ONNX
    model (~570 MB) into moons/io/data/ . Allow a few minutes.
  - The GUI starts the Moons as background processes; logs are visible in
    the "Service Detail" panel.

EOF
}

main() {
    install_system_deps
    build_rust_workspace
    build_tauri_gui
    setup_amalthea_venv
    setup_google_ads_venv
    setup_meta_ads_venv
    bootstrap_mcp_json
    print_next_steps
}

main "$@"
