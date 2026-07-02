#!/usr/bin/env bash
# Installation script for ccq — the read-only Claude Code transcript query CLI
#
# Downloads a prebuilt binary from the latest GitHub Release for the host
# platform. Falls back to `cargo build` from source if downloads fail and
# cargo is available, or when CCQ_FROM_SOURCE=1.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/joeaguilar/ccq/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/joeaguilar/ccq/main/install.sh | bash -s -- --update
#   ./install.sh
#   ./install.sh --update
#
# Environment overrides:
#   CCQ_VERSION       Pin a specific release tag (e.g. v0.1.0). Defaults to latest.
#   CCQ_INSTALL_DIR   Install directory. Defaults to $HOME/.local/bin (or
#                     an existing ccq on PATH, then $HOME/.cargo/bin when it
#                     already exists in PATH).
#   CCQ_FROM_SOURCE   If 1, skip prebuilt download and build with cargo.
#   CCQ_REPO          GitHub repo slug. Defaults to joeaguilar/ccq.

set -euo pipefail

REPO="${CCQ_REPO:-joeaguilar/ccq}"
VERSION="${CCQ_VERSION:-}"
FROM_SOURCE="${CCQ_FROM_SOURCE:-0}"
ACTION="install"

# Colors (suppressed when not a tty)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[1;33m'
    BLUE='\033[0;34m'
    NC='\033[0m'
else
    RED=''; GREEN=''; YELLOW=''; BLUE=''; NC=''
fi

info()    { printf "${BLUE}ℹ${NC} %s\n" "$1"; }
success() { printf "${GREEN}✓${NC} %s\n" "$1"; }
warning() { printf "${YELLOW}⚠${NC} %s\n" "$1" >&2; }
error()   { printf "${RED}✗${NC} %s\n" "$1" >&2; }

usage() {
    cat <<EOF
Usage:
  ./install.sh [--update]
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | bash -s -- [--update]

Options:
  --update    Update an existing ccq install if found on PATH; otherwise install it.
  -h, --help  Show this help text.

Environment:
  CCQ_VERSION       Pin a specific release tag (defaults to latest).
  CCQ_INSTALL_DIR   Override the install directory.
  CCQ_FROM_SOURCE   Set to 1 to build from source in this repo.
  CCQ_REPO          Override the GitHub repo slug.
EOF
}

parse_args() {
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --update|update)
                ACTION="update"
                ;;
            --install|install)
                ACTION="install"
                ;;
            -h|--help)
                usage
                exit 0
                ;;
            *)
                error "Unknown argument: $1"
                usage >&2
                exit 1
                ;;
        esac
        shift
    done
}

# ---- Platform detection -----------------------------------------------------

detect_target() {
    local uname_s uname_m
    uname_s=$(uname -s)
    uname_m=$(uname -m)

    case "$uname_s" in
        Darwin)
            case "$uname_m" in
                arm64|aarch64) echo "aarch64-apple-darwin" ;;
                x86_64)        echo "x86_64-apple-darwin" ;;
                *) error "Unsupported macOS architecture: $uname_m"; return 1 ;;
            esac
            ;;
        Linux)
            # Default to the fully-static musl build on x86_64 — it runs on
            # any distro regardless of glibc version. The glibc artifact is
            # still published for users who explicitly want it.
            case "$uname_m" in
                x86_64)        echo "x86_64-unknown-linux-musl" ;;
                aarch64|arm64) echo "aarch64-unknown-linux-gnu" ;;
                *) error "Unsupported Linux architecture: $uname_m"; return 1 ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*)
            error "Detected Windows shell. Please use install.ps1 instead:"
            error "  iwr -useb https://raw.githubusercontent.com/${REPO}/main/install.ps1 | iex"
            return 1
            ;;
        *)
            error "Unsupported OS: $uname_s"; return 1 ;;
    esac
}

# ---- Download helpers -------------------------------------------------------

http_get() {
    local url="$1" out="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$url" -o "$out"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$out" "$url"
    else
        error "Need curl or wget to download releases."
        return 1
    fi
}

resolve_version() {
    if [ -n "$VERSION" ]; then
        echo "$VERSION"
        return 0
    fi
    # Use the GH redirect on /releases/latest to avoid API rate limits and
    # the need for a token.
    local url="https://github.com/${REPO}/releases/latest"
    local resolved
    if command -v curl >/dev/null 2>&1; then
        resolved=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$url")
    elif command -v wget >/dev/null 2>&1; then
        resolved=$(wget --max-redirect=5 -S --spider "$url" 2>&1 | awk '/^  Location: /{loc=$2} END{print loc}')
    else
        error "Need curl or wget to query the latest release."
        return 1
    fi
    # Strip everything up to the last '/'
    echo "${resolved##*/}"
}

verify_checksum() {
    local file="$1" sumfile="$2"
    local expected actual
    expected=$(awk '{print $1}' "$sumfile" | head -1)
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$file" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
        actual=$(shasum -a 256 "$file" | awk '{print $1}')
    else
        warning "No sha256sum/shasum found — skipping checksum verification."
        return 0
    fi
    if [ "$expected" != "$actual" ]; then
        error "Checksum mismatch: expected $expected, got $actual"
        return 1
    fi
    success "Checksum verified."
}

# ---- Install dir selection --------------------------------------------------

choose_install_dir() {
    if [ -n "${CCQ_INSTALL_DIR:-}" ]; then
        echo "${CCQ_INSTALL_DIR/#\~/$HOME}"
        return
    fi
    # Updating should replace the binary the shell already resolves, when one
    # exists. This avoids installing a fresh copy behind a stale PATH entry.
    local existing
    existing=$(command -v ccq 2>/dev/null || true)
    if [ -n "$existing" ] && [ -f "$existing" ]; then
        dirname "$existing"
        return
    fi
    # If ~/.cargo/bin exists and is in PATH, prefer it (Rust users).
    case ":$PATH:" in
        *":$HOME/.cargo/bin:"*)
            if [ -d "$HOME/.cargo/bin" ]; then
                echo "$HOME/.cargo/bin"
                return
            fi
            ;;
    esac
    echo "$HOME/.local/bin"
}

is_in_path() {
    case ":$PATH:" in
        *":$1:"*) return 0 ;;
        *)        return 1 ;;
    esac
}

existing_ccq_binary() {
    local install_dir="$1"
    if [ -x "$install_dir/ccq" ]; then
        echo "$install_dir/ccq"
        return
    fi
    local existing
    existing=$(command -v ccq 2>/dev/null || true)
    if [ -n "$existing" ] && [ -x "$existing" ]; then
        echo "$existing"
    fi
}

print_existing_version() {
    local install_dir="$1" existing version
    existing=$(existing_ccq_binary "$install_dir")
    if [ -z "$existing" ]; then
        return
    fi
    version=$("$existing" --version 2>/dev/null || true)
    if [ -n "$version" ]; then
        info "Current install: $version ($existing)"
    else
        info "Current install: $existing"
    fi
}

# ---- Install paths ----------------------------------------------------------

install_from_release() {
    local target tag asset_base archive_url checksum_url
    target=$(detect_target) || return 1
    info "Detected target: $target"

    tag=$(resolve_version)
    if [ -z "$tag" ] || [ "$tag" = "releases" ]; then
        error "Could not resolve latest release tag."
        return 1
    fi
    info "Release: $tag"

    asset_base="ccq-${tag}-${target}"
    archive_url="https://github.com/${REPO}/releases/download/${tag}/${asset_base}.tar.gz"
    checksum_url="${archive_url}.sha256"

    local tmpdir
    tmpdir=$(mktemp -d)
    trap 'rm -rf "$tmpdir"' RETURN

    info "Downloading ${asset_base}.tar.gz"
    if ! http_get "$archive_url" "$tmpdir/${asset_base}.tar.gz"; then
        error "Download failed: $archive_url"
        return 1
    fi
    if ! http_get "$checksum_url" "$tmpdir/${asset_base}.tar.gz.sha256"; then
        warning "Checksum file not found at $checksum_url"
    else
        verify_checksum "$tmpdir/${asset_base}.tar.gz" "$tmpdir/${asset_base}.tar.gz.sha256" || return 1
    fi

    info "Extracting…"
    tar -xzf "$tmpdir/${asset_base}.tar.gz" -C "$tmpdir"
    if [ ! -f "$tmpdir/${asset_base}/ccq" ]; then
        error "Extracted archive is missing the ccq binary."
        return 1
    fi

    local install_dir existing_before need_sudo=0
    install_dir=$(choose_install_dir)
    existing_before=$(existing_ccq_binary "$install_dir")
    print_existing_version "$install_dir"
    mkdir -p "$install_dir" 2>/dev/null || true
    if [ ! -w "$install_dir" ]; then
        need_sudo=1
    fi

    if [ -n "$existing_before" ]; then
        info "Updating $install_dir/ccq"
    else
        info "Installing to $install_dir"
    fi
    if [ "$need_sudo" = 1 ]; then
        sudo install -m 0755 "$tmpdir/${asset_base}/ccq" "$install_dir/ccq"
    else
        install -m 0755 "$tmpdir/${asset_base}/ccq" "$install_dir/ccq"
    fi
    if [ -n "$existing_before" ]; then
        success "Updated $install_dir/ccq"
    else
        success "Installed $install_dir/ccq"
    fi

    if ! is_in_path "$install_dir"; then
        warning "$install_dir is not in your PATH."
        echo "  Add it with:"
        echo "    echo 'export PATH=\"$install_dir:\$PATH\"' >> ~/.bashrc   # or ~/.zshrc"
    fi

    echo
    "$install_dir/ccq" --version 2>/dev/null || true
    return 0
}

install_from_source() {
    if ! command -v cargo >/dev/null 2>&1; then
        error "cargo is not installed — cannot build from source."
        error "Install Rust from https://rustup.rs/ and re-run, or set a working network connection."
        return 1
    fi
    if [ ! -f Cargo.toml ]; then
        error "Source build requires running this script from inside a cloned ccq repo."
        error "Try: git clone https://github.com/${REPO} && cd ccq && ./install.sh"
        return 1
    fi
    info "Building from source with cargo…"
    cargo build --release
    local install_dir existing_before need_sudo=0
    install_dir=$(choose_install_dir)
    existing_before=$(existing_ccq_binary "$install_dir")
    print_existing_version "$install_dir"
    mkdir -p "$install_dir" 2>/dev/null || true
    if [ ! -w "$install_dir" ]; then
        need_sudo=1
    fi
    if [ "$need_sudo" = 1 ]; then
        sudo install -m 0755 target/release/ccq "$install_dir/ccq"
    else
        install -m 0755 target/release/ccq "$install_dir/ccq"
    fi
    if [ -n "$existing_before" ]; then
        success "Updated $install_dir/ccq"
    else
        success "Installed $install_dir/ccq"
    fi
    if ! is_in_path "$install_dir"; then
        warning "$install_dir is not in your PATH."
        echo "  export PATH=\"$install_dir:\$PATH\""
    fi
}

# ---- Main -------------------------------------------------------------------

main() {
    parse_args "$@"

    echo
    if [ "$ACTION" = "update" ]; then
        info "Updating ccq — the read-only Claude Code transcript query CLI"
    else
        info "Installing ccq — the read-only Claude Code transcript query CLI"
    fi
    echo

    if [ "$FROM_SOURCE" = "1" ]; then
        install_from_source
    elif install_from_release; then
        :
    else
        warning "Prebuilt install failed — falling back to cargo build."
        install_from_source
    fi

    echo
    success "Done."
    echo
    info "Quick start:"
    echo "  ccq projects                         # list transcript projects"
    echo "  ccq stats --since 7d                 # corpus rollup, last week"
    echo "  ccq bash -p . --count --by command   # top commands in this project"
    echo "  ccq --help                           # all commands"
    echo
}

main "$@"
