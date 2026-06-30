#!/usr/bin/env bash
set -e

REPO="ozten/MatchyMatchy"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info()    { echo -e "${BLUE}==>${NC} $1"; }
log_success() { echo -e "${GREEN}==>${NC} $1"; }
log_warning() { echo -e "${YELLOW}==>${NC} $1"; }
log_error()   { echo -e "${RED}Error:${NC} $1" >&2; }

detect_platform() {
    local os arch

    case "$(uname -s)" in
        Darwin) os="darwin" ;;
        Linux)  os="linux" ;;
        *)
            log_error "Unsupported operating system: $(uname -s)"
            exit 1
            ;;
    esac

    case "$(uname -m)" in
        x86_64|amd64)   arch="amd64" ;;
        aarch64|arm64)  arch="arm64" ;;
        *)
            log_error "Unsupported architecture: $(uname -m)"
            exit 1
            ;;
    esac

    echo "${os}_${arch}"
}

fetch() {
    local url=$1
    if command -v curl &>/dev/null; then
        curl -fsSL "$url"
    elif command -v wget &>/dev/null; then
        wget -qO- "$url"
    else
        log_error "Neither curl nor wget found."
        exit 1
    fi
}

download() {
    local url=$1 dest=$2
    if command -v curl &>/dev/null; then
        curl -fsSL -o "$dest" "$url"
    else
        wget -q -O "$dest" "$url"
    fi
}

resign_for_macos() {
    [[ "$(uname -s)" != "Darwin" ]] && return 0
    command -v codesign &>/dev/null || return 0

    log_info "Re-signing $1 for macOS..."
    codesign --remove-signature "$1" 2>/dev/null || true
    codesign --force --sign - "$1" 2>/dev/null && log_success "Binary re-signed" || true
}

# place_file <src-tmp> <dest-dir>/<name> — move into place with sudo fallback.
place() {
    local src=$1 dest=$2
    if [[ -w "$(dirname "$dest")" ]]; then
        mv "$src" "$dest"
    else
        sudo mv "$src" "$dest"
    fi
}

main() {
    echo ""
    echo "matchy Installer"
    echo ""

    local platform
    platform=$(detect_platform)
    log_info "Platform: $platform"

    local version release_json
    if [[ -n "${MATCHY_VERSION:-}" ]]; then
        version="v${MATCHY_VERSION#v}"
        log_info "Requested version: $version"
        release_json=$(fetch "https://api.github.com/repos/${REPO}/releases/tags/${version}")
    else
        log_info "Fetching latest release..."
        release_json=$(fetch "https://api.github.com/repos/${REPO}/releases/latest")
        version=$(echo "$release_json" | grep '"tag_name"' | sed -E 's/.*"tag_name": "([^"]+)".*/\1/')
    fi

    if [[ -z "$version" ]]; then
        log_error "Failed to determine version"
        exit 1
    fi
    log_info "Installing matchy $version"

    local install_dir="/usr/local/bin"
    if [[ ! -w "$install_dir" ]]; then
        install_dir="$HOME/.local/bin"
        mkdir -p "$install_dir"
    fi

    local base="https://github.com/${REPO}/releases/download/${version}"
    local version_num="${version#v}"
    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "$tmp_dir"' EXIT

    # --- Artifact 1: the matchy binary (analyze layer) ---
    local archive="matchy_${version_num}_${platform}.tar.gz"
    if ! echo "$release_json" | grep -Fq "\"name\": \"$archive\""; then
        log_error "No prebuilt matchy for $platform in release $version"
        echo "  Available at: https://github.com/${REPO}/releases/tag/${version}"
        exit 1
    fi
    log_info "Downloading $archive..."
    download "$base/$archive" "$tmp_dir/$archive"
    tar -xzf "$tmp_dir/$archive" -C "$tmp_dir"
    chmod +x "$tmp_dir/matchy"
    place "$tmp_dir/matchy" "$install_dir/matchy"
    resign_for_macos "$install_dir/matchy"
    log_success "Installed matchy to $install_dir/matchy"

    # --- Artifact 2: capture.cjs (capture layer, run by host Node) ---
    # Lives alongside the binary; matchy resolves it relative to its own path.
    log_info "Downloading capture.cjs..."
    download "$base/capture.cjs" "$tmp_dir/capture.cjs"
    place "$tmp_dir/capture.cjs" "$install_dir/capture.cjs"
    log_success "Installed capture.cjs to $install_dir/capture.cjs"

    if [[ ":$PATH:" != *":$install_dir:"* ]]; then
        log_warning "$install_dir is not in your PATH"
        echo "  Add to your shell profile:  export PATH=\"\$PATH:$install_dir\""
    fi

    echo ""
    log_success "Artifacts installed."
    command -v matchy &>/dev/null && matchy --version 2>/dev/null || true

    # --- Host runtime prerequisites (we never install these) ---
    echo ""
    log_info "matchy runs the capture layer with your system Node + Playwright."
    log_info "These are host prerequisites — install them once:"
    echo "    1) Node.js >= 20                       (https://nodejs.org)"
    echo "    2) npm install -g playwright@1.60.0    # pinned + global, so capture.cjs can resolve it"
    echo "    3) npx playwright install chromium     # pulls the Chromium build that 1.60.0 pins"
    echo "    4) Chromium's system libraries         # the browser downloads but won't LAUNCH without them"
    echo "         Debian/Ubuntu:     sudo npx playwright install-deps chromium"
    echo "         Amazon Linux/RHEL: sudo dnf install -y nss nspr atk at-spi2-atk at-spi2-core \\"
    echo "                              cups-libs libdrm libxkbcommon libXcomposite libXdamage \\"
    echo "                              libXext libXfixes libXrandr libgbm mesa-libgbm libX11 \\"
    echo "                              libxcb pango cairo alsa-lib"
    echo ""
    log_info "Then verify everything is wired up:"
    echo "    matchy doctor"
    log_info "doctor checks each requirement and prints the exact fix for anything missing."
    log_warning "If doctor says 'Chromium build NNNN not found' but the build IS downloaded,"
    log_warning "the real cause is almost always missing system libraries (step 4)."
}

main "$@"
