#!/usr/bin/env bash
set -euo pipefail

# Generic Arshy installer. It installs only the two release binaries and never
# edits an Agent config, shell profile, GUI PATH, or workspace file.

VERSION="v0.1.0-dev.1"
INSTALL_DIR="${HOME}/.local/bin"
DRY_RUN=0

usage() {
    cat <<'EOF'
Usage: install.sh [--version TAG] [--install-dir DIR] [--dry-run]

Downloads a verified Arshy release for the current OS and architecture.
After installation, configure any MCP-capable client with:
  arshy mcp config
EOF
}

while (($# > 0)); do
    case "$1" in
        --version)
            [[ $# -ge 2 ]] || { echo "--version requires a tag" >&2; exit 2; }
            VERSION="$2"
            shift 2
            ;;
        --install-dir)
            [[ $# -ge 2 ]] || { echo "--install-dir requires a path" >&2; exit 2; }
            INSTALL_DIR="$2"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

OS="$(uname -s)"
ARCH="$(uname -m)"
case "${OS}-${ARCH}" in
    Darwin-arm64|Darwin-aarch64) TARGET="aarch64-apple-darwin" ;;
    Darwin-x86_64) TARGET="x86_64-apple-darwin" ;;
    Linux-x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
    Linux-arm64|Linux-aarch64) TARGET="aarch64-unknown-linux-gnu" ;;
    *)
        echo "unsupported platform: ${OS} (${ARCH})" >&2
        exit 1
        ;;
esac

BASE_URL="https://github.com/iZoy/Arshy/releases/download/${VERSION}"
ARCHIVE="arshy-${VERSION}-${TARGET}.tar.gz"
CHECKSUM="${ARCHIVE}.sha256"

if ((DRY_RUN)); then
    printf 'version:     %s\nplatform:    %s\narchive:     %s/%s\ninstall dir: %s\n' \
        "$VERSION" "$TARGET" "$BASE_URL" "$ARCHIVE" "$INSTALL_DIR"
    exit 0
fi

download() {
    local url="$1" output="$2"
    if command -v curl >/dev/null 2>&1; then
        curl --fail --location --silent --show-error "$url" --output "$output"
    elif command -v wget >/dev/null 2>&1; then
        wget --https-only --quiet --output-document="$output" "$url"
    else
        echo "curl or wget is required" >&2
        exit 1
    fi
}

verify_checksum() {
    local checksum_file="$1" archive_file="$2"
    if command -v shasum >/dev/null 2>&1; then
        (cd "$(dirname "$archive_file")" && shasum -a 256 -c "$(basename "$checksum_file")")
    elif command -v sha256sum >/dev/null 2>&1; then
        (cd "$(dirname "$archive_file")" && sha256sum --check "$(basename "$checksum_file")")
    else
        echo "shasum or sha256sum is required to verify the release" >&2
        exit 1
    fi
}

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/arshy-install.XXXXXX")"
cleanup() { rm -rf "$TMP_DIR"; }
trap cleanup EXIT

echo "Downloading Arshy ${VERSION} (${TARGET})..."
download "${BASE_URL}/${ARCHIVE}" "${TMP_DIR}/${ARCHIVE}"
download "${BASE_URL}/${CHECKSUM}" "${TMP_DIR}/${CHECKSUM}"
verify_checksum "${TMP_DIR}/${CHECKSUM}" "${TMP_DIR}/${ARCHIVE}"

mkdir -p "${TMP_DIR}/extracted" "${INSTALL_DIR}"
tar -xzf "${TMP_DIR}/${ARCHIVE}" -C "${TMP_DIR}/extracted"
[[ -x "${TMP_DIR}/extracted/arshy" ]] || { echo "archive is missing arshy" >&2; exit 1; }
[[ -x "${TMP_DIR}/extracted/arshyd" ]] || { echo "archive is missing arshyd" >&2; exit 1; }

# Stage both files before replacing either installed binary. A failed copy
# restores the previous pair so an interrupted upgrade cannot split versions.
STAGE="${INSTALL_DIR}/.arshy-stage.$$"
BACKUP="${TMP_DIR}/backup"
mkdir -p "${STAGE}" "${BACKUP}"
cp "${TMP_DIR}/extracted/arshy" "${STAGE}/arshy"
cp "${TMP_DIR}/extracted/arshyd" "${STAGE}/arshyd"
chmod 755 "${STAGE}/arshy" "${STAGE}/arshyd"

had_arshy=0
had_arshyd=0
[[ -e "${INSTALL_DIR}/arshy" ]] && { cp "${INSTALL_DIR}/arshy" "${BACKUP}/arshy"; had_arshy=1; }
[[ -e "${INSTALL_DIR}/arshyd" ]] && { cp "${INSTALL_DIR}/arshyd" "${BACKUP}/arshyd"; had_arshyd=1; }
rollback() {
    if [[ -f "${BACKUP}/arshy" ]]; then cp "${BACKUP}/arshy" "${INSTALL_DIR}/arshy"; elif ((had_arshy == 0)); then rm -f "${INSTALL_DIR}/arshy"; fi
    if [[ -f "${BACKUP}/arshyd" ]]; then cp "${BACKUP}/arshyd" "${INSTALL_DIR}/arshyd"; elif ((had_arshyd == 0)); then rm -f "${INSTALL_DIR}/arshyd"; fi
    rm -rf "${STAGE}"
}
trap rollback ERR
mv -f "${STAGE}/arshy" "${INSTALL_DIR}/arshy"
mv -f "${STAGE}/arshyd" "${INSTALL_DIR}/arshyd"
rm -rf "${STAGE}"
trap - ERR

echo "Installed arshy and arshyd in ${INSTALL_DIR}."
echo "If needed, add this directory to PATH:"
echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
echo "Generic MCP configuration:"
"$INSTALL_DIR/arshy" mcp config
