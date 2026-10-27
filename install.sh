#!/usr/bin/env bash
#
# agent-switch (ags) installer & uninstaller
# Supports: Linux (x86_64, aarch64 - musl static) and macOS (x86_64, aarch64)
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.sh | bash -s -- --uninstall
#   curl -fsSL https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.sh | bash -s -- --uninstall --clean-data
#   curl -fsSL https://raw.githubusercontent.com/Skyxim/agent-switch/main/install.sh | bash -s -- --version 0.1.0
#

set -euo pipefail

REPO="Skyxim/agent-switch"
GITHUB_URL="https://github.com/${REPO}"
GITHUB_API="https://api.github.com/repos/${REPO}"
BIN_NAME="ags"

# Colors & Formatting
if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  BOLD='\033[1m'
  DIM='\033[2m'
  RED='\033[0;31m'
  GREEN='\033[0;32m'
  YELLOW='\033[0;33m'
  BLUE='\033[0;34m'
  CYAN='\033[0;36m'
  RESET='\033[0m'
else
  BOLD=''
  DIM=''
  RED=''
  GREEN=''
  YELLOW=''
  BLUE=''
  CYAN=''
  RESET=''
fi

info() {
  printf "${CYAN}info:${RESET} %s\n" "$*"
}

success() {
  printf "${GREEN}success:${RESET} %s\n" "$*"
}

warn() {
  printf "${YELLOW}warning:${RESET} %s\n" "$*"
}

error() {
  printf "${RED}error:${RESET} %s\n" "$*" >&2
}

step() {
  printf "${BOLD}${BLUE}==>${RESET} ${BOLD}%s${RESET}\n" "$*"
}

print_banner() {
  cat <<'EOF'
    ___                    __     ______          _ __       __  
   /   | ____ ____  ____  / /_   / ___/      __  (_) /______/ /_ 
  / /| |/ __ `/ _ \/ __ \/ __/   \__ \ | | /| / / / __/ ___/ __ \
 / ___ / /_/ /  __/ / / / /_    ___/ / | |/ |/ / / /_/ /__/ / / /
/_/  |_\__, /\___/_/ /_/\__/   /____/  |__/|__/_/\__/\___/_/ /_/ 
      /____/                                                     
EOF
  printf "${DIM}Fast, encrypted CLI switcher for AI agent credentials${RESET}\n\n"
}

show_help() {
  cat <<EOF
Usage: install.sh [OPTIONS]

Options:
  -u, --uninstall        Uninstall agent-switch (ags) binary
      --clean-data       Remove configuration & data directory (~/.config/agent-switch) during uninstall
  -v, --version <VER>    Specify version to install (e.g. 0.1.0 or v0.1.0, default: latest)
  -h, --help             Show this help message
EOF
}

# Temporary workspace cleanup trap
TMP_DIR=""
cleanup() {
  if [ -n "${TMP_DIR:-}" ] && [ -d "${TMP_DIR}" ]; then
    rm -rf "${TMP_DIR}"
  fi
}
trap cleanup EXIT INT TERM

# Uninstall logic
do_uninstall() {
  step "Uninstalling agent-switch (${BIN_NAME})..."
  local removed=0

  # Known standard install locations
  local candidates=(
    "/usr/local/bin/${BIN_NAME}"
    "${HOME}/.local/bin/${BIN_NAME}"
  )

  # Also detect if binary is located elsewhere in PATH
  local existing_path
  existing_path=$(command -v "${BIN_NAME}" 2>/dev/null || true)
  if [ -n "${existing_path}" ]; then
    candidates+=("${existing_path}")
  fi

  # Deduplicate and remove binaries
  local seen=()
  for bin_path in "${candidates[@]}"; do
    # Skip if empty or already checked
    if [ -z "${bin_path}" ]; then continue; fi
    local already_checked=0
    for s in "${seen[@]:-}"; do
      if [ "${s}" = "${bin_path}" ]; then
        already_checked=1
        break
      fi
    done
    if [ "${already_checked}" -eq 1 ]; then continue; fi
    seen+=("${bin_path}")

    if [ -e "${bin_path}" ] || [ -L "${bin_path}" ]; then
      if [ -w "${bin_path}" ] || [ -w "$(dirname "${bin_path}")" ]; then
        rm -f "${bin_path}"
        info "Removed binary: ${bin_path}"
        removed=1
      else
        if command -v sudo >/dev/null 2>&1 && [ -t 0 ]; then
          info "Requesting sudo permissions to remove ${bin_path}..."
          sudo rm -f "${bin_path}"
          removed=1
        else
          warn "Permission denied to remove ${bin_path}. Please remove manually: sudo rm ${bin_path}"
        fi
      fi
    fi
  done

  # Clean data directory if requested
  if [ "${CLEAN_DATA}" -eq 1 ]; then
    local data_dir="${AGS_HOME:-${HOME}/.config/agent-switch}"
    if [ -d "${data_dir}" ]; then
      rm -rf "${data_dir}"
      info "Removed configuration and data directory: ${data_dir}"
    else
      info "Data directory ${data_dir} does not exist."
    fi
  else
    local data_dir="${AGS_HOME:-${HOME}/.config/agent-switch}"
    if [ -d "${data_dir}" ]; then
      info "Configuration & data directory preserved: ${data_dir}"
      info "Use --clean-data to remove it."
    fi
  fi

  if [ "${removed}" -eq 1 ]; then
    success "agent-switch has been successfully uninstalled."
  else
    warn "No installed binary found for '${BIN_NAME}'."
  fi
}

# Download helper supporting curl and wget
download_file() {
  local url="$1"
  local dest="$2"

  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "${url}" -o "${dest}"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "${dest}" "${url}"
  else
    error "Neither curl nor wget was found. Please install curl or wget."
    exit 1
  fi
}

# Resolve latest release tag from GitHub
resolve_latest_version() {
  local tag=""

  # 1. Try GitHub Releases API
  if command -v curl >/dev/null 2>&1; then
    tag=$(curl -sSfL "${GITHUB_API}/releases/latest" 2>/dev/null | grep -o '"tag_name": *"[^"]*"' | head -n 1 | cut -d'"' -f4 || true)
  elif command -v wget >/dev/null 2>&1; then
    tag=$(wget -qO- "${GITHUB_API}/releases/latest" 2>/dev/null | grep -o '"tag_name": *"[^"]*"' | head -n 1 | cut -d'"' -f4 || true)
  fi

  if [ -n "${tag}" ]; then
    echo "${tag}"
    return 0
  fi

  # 2. Fallback: check redirect header of /releases/latest
  # When a release exists, it redirects to https://github.com/<repo>/releases/tag/<tag>
  # When no release exists, GitHub redirects to https://github.com/<repo>/releases
  if command -v curl >/dev/null 2>&1; then
    local redirect_loc
    redirect_loc=$(curl -sSfIL "${GITHUB_URL}/releases/latest" 2>/dev/null | grep -i "^location:" | head -n 1 | tr -d '\r\n' || true)
    if [[ "${redirect_loc}" =~ /releases/tag/ ]]; then
      tag="${redirect_loc##*/}"
    fi
  fi

  if [ -n "${tag}" ] && [ "${tag}" != "releases" ]; then
    echo "${tag}"
    return 0
  fi

  return 1
}

# Command-line flags
UNINSTALL=0
CLEAN_DATA=0
SPECIFIED_VERSION=""

while [ $# -gt 0 ]; do
  case "$1" in
    -u|--uninstall)
      UNINSTALL=1
      shift
      ;;
    --clean-data)
      CLEAN_DATA=1
      shift
      ;;
    -v|--version)
      if [ -n "${2:-}" ] && [ "${2#-}" = "$2" ]; then
        SPECIFIED_VERSION="$2"
        shift 2
      else
        error "Option $1 requires a version argument (e.g. 0.1.0 or v0.1.0)"
        exit 1
      fi
      ;;
    --version=*)
      SPECIFIED_VERSION="${1#*=}"
      shift
      ;;
    -h|--help)
      show_help
      exit 0
      ;;
    *)
      error "Unknown option: $1"
      show_help
      exit 1
      ;;
  esac
done

print_banner

if [ "${UNINSTALL}" -eq 1 ]; then
  do_uninstall
  exit 0
fi

step "Detecting platform and architecture..."

RAW_OS="$(uname -s)"
RAW_ARCH="$(uname -m)"

case "${RAW_OS}" in
  Linux|linux)
    OS="linux"
    ;;
  Darwin|darwin)
    OS="apple-darwin"
    ;;
  *)
    error "Unsupported operating system: ${RAW_OS}"
    info "agent-switch install script currently supports Linux and macOS (Darwin)."
    exit 1
    ;;
esac

case "${RAW_ARCH}" in
  x86_64|amd64)
    ARCH="x86_64"
    ;;
  aarch64|arm64)
    ARCH="aarch64"
    ;;
  *)
    error "Unsupported architecture: ${RAW_ARCH}"
    info "agent-switch currently supports x86_64 and aarch64 (arm64)."
    exit 1
    ;;
esac

# Target matching:
# Linux: prefer static musl (x86_64-unknown-linux-musl / aarch64-unknown-linux-musl)
# macOS: aarch64-apple-darwin / x86_64-apple-darwin
TARGET_TRIPLE=""
FALLBACK_TRIPLE=""

if [ "${OS}" = "linux" ]; then
  TARGET_TRIPLE="${ARCH}-unknown-linux-musl"
  FALLBACK_TRIPLE="${ARCH}-unknown-linux-gnu"
  info "Detected Linux (${ARCH}), using static musl binary for zero-dependency compatibility."
elif [ "${OS}" = "apple-darwin" ]; then
  TARGET_TRIPLE="${ARCH}-apple-darwin"
  FALLBACK_TRIPLE=""
  info "Detected macOS (${ARCH})."
fi

step "Resolving agent-switch release version..."

TAG=""
VERSION=""

if [ -n "${SPECIFIED_VERSION}" ]; then
  VERSION="${SPECIFIED_VERSION#v}"
  TAG="v${VERSION}"
  info "Using specified version: ${TAG}"
else
  if ! TAG=$(resolve_latest_version); then
    error "Failed to retrieve the latest version of ${REPO} from GitHub."
    info "You can specify a target version manually: install.sh --version <VER>"
    exit 1
  fi
  VERSION="${TAG#v}"
  info "Latest release version: ${TAG}"
fi

# Prepare temporary directory
TMP_DIR="$(mktemp -d 2>/dev/null || mktemp -d -t 'ags_install_XXXXXX')"

step "Downloading release asset..."

# List candidate URLs in priority order
CANDIDATE_URLS=(
  "${GITHUB_URL}/releases/download/${TAG}/ags-${VERSION}-${TARGET_TRIPLE}.tar.gz"
  "${GITHUB_URL}/releases/download/${TAG}/ags-${TAG}-${TARGET_TRIPLE}.tar.gz"
)

if [ -n "${FALLBACK_TRIPLE}" ]; then
  CANDIDATE_URLS+=(
    "${GITHUB_URL}/releases/download/${TAG}/ags-${VERSION}-${FALLBACK_TRIPLE}.tar.gz"
    "${GITHUB_URL}/releases/download/${TAG}/ags-${TAG}-${FALLBACK_TRIPLE}.tar.gz"
  )
fi

DOWNLOAD_SUCCESS=0
DOWNLOADED_FILE="${TMP_DIR}/downloaded_asset"

for url in "${CANDIDATE_URLS[@]}"; do
  info "Trying asset: ${url}"
  if download_file "${url}" "${DOWNLOADED_FILE}" 2>/dev/null; then
    DOWNLOAD_SUCCESS=1
    success "Successfully downloaded asset from: ${url}"
    break
  fi
done

if [ "${DOWNLOAD_SUCCESS}" -ne 1 ]; then
  error "Failed to download release asset for ${TARGET_TRIPLE} (${TAG})."
  info "Please verify that a release exists at: ${GITHUB_URL}/releases"
  exit 1
fi

step "Extracting binary..."

EXTRACT_DIR="${TMP_DIR}/extract"
mkdir -p "${EXTRACT_DIR}"

FOUND_BIN=""

tar -xzf "${DOWNLOADED_FILE}" -C "${EXTRACT_DIR}"
FOUND_BIN=$(find "${EXTRACT_DIR}" -type f -name "${BIN_NAME}" -perm -111 2>/dev/null | head -n 1 || true)
if [ -z "${FOUND_BIN}" ]; then
  FOUND_BIN=$(find "${EXTRACT_DIR}" -type f -name "${BIN_NAME}" 2>/dev/null | head -n 1 || true)
fi

if [ -z "${FOUND_BIN}" ] || [ ! -f "${FOUND_BIN}" ]; then
  error "Could not find '${BIN_NAME}' binary in downloaded asset."
  exit 1
fi

chmod +x "${FOUND_BIN}"

step "Installing binary..."

# Installation destination:
# Priority 1: /usr/local/bin if writable
# Priority 2: ~/.local/bin safely without requiring root/sudo
INSTALL_DIR=""

if [ -d "/usr/local/bin" ] && [ -w "/usr/local/bin" ]; then
  INSTALL_DIR="/usr/local/bin"
else
  INSTALL_DIR="${HOME}/.local/bin"
  mkdir -p "${INSTALL_DIR}"
fi

TARGET_BIN="${INSTALL_DIR}/${BIN_NAME}"
cp -f "${FOUND_BIN}" "${TARGET_BIN}"
chmod 755 "${TARGET_BIN}"

success "Installed ${BIN_NAME} to ${TARGET_BIN}"

# Verify PATH
IN_PATH=0
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*)
    IN_PATH=1
    ;;
  *)
    IN_PATH=0
    ;;
esac

echo ""
if [ "${IN_PATH}" -eq 1 ]; then
  success "Installation complete! '${BIN_NAME}' is ready to use in your current PATH."
else
  warn "'${INSTALL_DIR}' is not currently in your PATH."
  echo ""
  echo "To use '${BIN_NAME}' directly, add it to your shell configuration file:"
  echo ""
  # Detect user's current shell
  SHELL_NAME="$(basename "${SHELL:-bash}")"
  case "${SHELL_NAME}" in
    zsh)
      printf "  ${BOLD}echo 'export PATH=\"%s:\$PATH\"' >> ~/.zshrc${RESET}\n" "${INSTALL_DIR}"
      printf "  ${BOLD}source ~/.zshrc${RESET}\n"
      ;;
    fish)
      printf "  ${BOLD}fish_add_path %s${RESET}\n" "${INSTALL_DIR}"
      ;;
    *)
      printf "  ${BOLD}echo 'export PATH=\"%s:\$PATH\"' >> ~/.bashrc${RESET}\n" "${INSTALL_DIR}"
      printf "  ${BOLD}source ~/.bashrc${RESET}\n"
      ;;
  esac
  echo ""
fi

# Print version and help info
if command -v "${TARGET_BIN}" >/dev/null 2>&1; then
  INSTALLED_VER=$("${TARGET_BIN}" --version 2>/dev/null || echo "${VERSION}")
  printf "${BOLD}${GREEN}✔ agent-switch (${INSTALLED_VER}) is ready!${RESET}\n\n"
fi

cat <<EOF
Quick Start:
  ags info                 # View environment and accounts status
  ags codex add [alias]    # Add current Codex account
  ags agy add [alias]      # Add current Antigravity account
  ags --help               # View all available commands

To uninstall:
  curl -fsSL https://raw.githubusercontent.com/${REPO}/main/install.sh | bash -s -- --uninstall
EOF
