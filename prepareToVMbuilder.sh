#!/usr/bin/env bash
# =============================================================================
#  prepareToVMbuilder.sh — bundle everything needed to build the VirtuaOS ISO
#  on a remote Debian 13 build VM.
#
#  build.sh bakes the Virtua panel from the FULL monorepo at the repo root
#  (PROJECT_ROOT = ../Virtua), NOT just VirtuaOS/. Copying only
#  VirtuaOS/ to a build box makes hook 0400 die with
#  "ENOENT /opt/auxinux-virtua/package.json". This script produces a single
#  .tar.gz holding exactly what the ISO build needs (monorepo source + the
#  VirtuaOS builder + any prebuilt .deb for the hybrid model), wrapped in a
#  top-level `auxinux/` dir.
#
#  Excluded (heavy / irrelevant to the ISO build): node_modules, .git,
#  Client_Desktop (~11 GB of Rust target/), external kernel projects, docs/,
#  img/, *.iso, *.pdf, prior build work dirs.
#
#  Usage:
#    ./prepareToVMbuilder.sh [output.tar.gz]
#
#  On the VM (extract at the home root, then build):
#    tar xzf virtuaos-build-bundle.tar.gz -C ~
#    cd ~/auxinux/VirtuaOS
#    sudo BUILD_CONTEXT=/mnt/build/virtuaos-build ./build.sh --native
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VIRTUA_SOURCE_DIR="${VIRTUA_SOURCE_DIR:-${SCRIPT_DIR}/../Virtua}"
PROJECT_ROOT="$(cd "${VIRTUA_SOURCE_DIR}" 2>/dev/null && pwd || true)"

WRAP="auxinux"   # top-level dir inside the tarball (=> ~/auxinux after extract)
OUT="${1:-${SCRIPT_DIR}/virtuaos-build-bundle.tar.gz}"

C='\033[0;36m'; G='\033[0;32m'; Y='\033[0;33m'; R='\033[0;31m'; N='\033[0m'
info(){ echo -e "${C}[INFO]${N} $*"; }
ok(){ echo -e "${G}[OK]${N} $*"; }
warn(){ echo -e "${Y}[WARN]${N} $*"; }
die(){ echo -e "${R}[ERR]${N} $*"; exit 1; }

command -v rsync >/dev/null 2>&1 || die "rsync is required."
command -v tar   >/dev/null 2>&1 || die "tar is required."

# Sanity: confirm we are really at the monorepo root.
for need in package.json apps packages lang tsconfig.base.json; do
    [[ -e "${PROJECT_ROOT}/${need}" ]] \
        || die "Expected ${need} under ${PROJECT_ROOT} — Virtua must be next to VirtuaOS."
done

# macOS tar: don't pollute the archive with AppleDouble (._*) / xattr noise.
export COPYFILE_DISABLE=1

STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT
DEST="${STAGE}/${WRAP}"
VIRTUA_DEST="${DEST}/Virtua"
VIRTUAOS_DEST="${DEST}/VirtuaOS"
mkdir -p "${VIRTUA_DEST}" "${VIRTUAOS_DEST}"

info "Staging monorepo source from ${PROJECT_ROOT}"
rsync -a \
    --exclude='.git' \
    --exclude='node_modules' \
    --exclude='dist' \
    --exclude='.DS_Store' \
    --exclude='._*' \
    --exclude='Client_Desktop' \
    --exclude='packaging' \
    --exclude='docs' \
    --exclude='img' \
    --exclude='.claude' \
    --exclude='*.iso' \
    --exclude='*.pdf' \
    --exclude='OS' \
    "${PROJECT_ROOT}/" "${VIRTUA_DEST}/"

# VirtuaOS is a sibling project and must travel with the Virtua source because
# the ISO build consumes its live-build tree and Rust installer.
rsync -a \
    --exclude='.DS_Store' \
    --exclude='._*' \
    --exclude='.build' \
    --exclude='output' \
    --exclude='workspace' \
    --exclude='*.iso' \
    --exclude='*.tar.gz' \
    --exclude='virtuaos-cli/target' \
    --exclude='virtuaos-installer/target' \
    "${SCRIPT_DIR}/" "${VIRTUAOS_DEST}/"

# Hybrid model: if prebuilt .deb(s) exist, ship them so build.sh's
# stage_local_debs() bakes + dpkg-registers the panel (enables `virtuaos update`).
# Optional — without them the ISO is still produced (bake-only).
DEB_SRC="${PROJECT_ROOT}/packaging/deb/out"
# Pull the current versions straight from the source of truth (package.json,
# Cargo.toml) so this always looks for the .deb matching what's checked out,
# not a hardcoded version that would silently go stale.
VIRTUA_VERSION="$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "${PROJECT_ROOT}/package.json" | head -1)"
CLI_VERSION="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "${SCRIPT_DIR}/virtuaos-cli/Cargo.toml" | head -1)"
CURRENT_DEBS=(
    "${DEB_SRC}/auxinux-virtua_${VIRTUA_VERSION}_amd64.deb"
    "${DEB_SRC}/virtuaos-cli_${CLI_VERSION}_amd64.deb"
)
FOUND_DEBS=()
for deb in "${CURRENT_DEBS[@]}"; do
    [[ -f "${deb}" ]] && FOUND_DEBS+=("${deb}")
done

if [[ ${#FOUND_DEBS[@]} -gt 0 ]]; then
    info "Including prebuilt .deb(s) for the hybrid model"
    mkdir -p "${VIRTUA_DEST}/packaging/deb/out"
    rsync -a "${FOUND_DEBS[@]}" "${VIRTUA_DEST}/packaging/deb/out/"
    ls -1 "${VIRTUA_DEST}/packaging/deb/out/"*.deb | sed 's,.*/,    ,'
fi
for deb in "${CURRENT_DEBS[@]}"; do
    [[ -f "${deb}" ]] || warn "Current package not found: $(basename "${deb}")"
done
[[ ${#FOUND_DEBS[@]} -gt 0 ]] || warn "No current .deb available; ISO will bake Virtua without dpkg registration."

info "Creating archive ${OUT}"
rm -f "${OUT}"
tar -C "${STAGE}" -czf "${OUT}" "${WRAP}"

ok "Bundle ready: ${OUT}  ($(du -sh "${OUT}" | cut -f1))"
echo
echo "Transfer it to the VM, e.g.:"
echo "    scp \"${OUT}\" debian@<vm-ip>:~/"
echo
echo "Then on the VM:"
echo "    tar xzf $(basename "${OUT}") -C ~"
echo "    cd ~/${WRAP}/VirtuaOS"
echo "    sudo BUILD_CONTEXT=/mnt/build/virtuaos-build ./build.sh --native"
