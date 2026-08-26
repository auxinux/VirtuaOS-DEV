#!/usr/bin/env bash
# =============================================================================
#  VirtuaOS 1.0.0 — ISO Builder
#  Builds a bootable x86_64 ISO based on Debian 13 (Trixie) with
#  AuxiNux Virtua Control pre-bundled and auto-configured on first boot.
#
#  ┌──────────────────────────────────────────────────────┐
#  │  macOS (Intel or Apple Silicon):                     │
#  │    Requires: Docker Desktop with "Use Rosetta"        │
#  │    Run:  ./build.sh                                  │
#  │                                                      │
#  │  Debian / Ubuntu Linux (native):                     │
#  │    Requires: live-build, xorriso, rsync              │
#  │    Run:  sudo ./build.sh --native                    │
#  └──────────────────────────────────────────────────────┘
# =============================================================================
set -euo pipefail

VIRTUAOS_VERSION="1.0.3-BETA"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
VIRTUA_SOURCE_DIR="${VIRTUA_SOURCE_DIR:-${SCRIPT_DIR}/../Virtua}"
PROJECT_ROOT="$(cd "${VIRTUA_SOURCE_DIR}" 2>/dev/null && pwd || true)"
# Where the finished ISO (and its short-lived staging dir) is written. Override
# to land it on a roomier disk, e.g.:
#   OUTPUT_DIR=/Volumes/INTERNEVM/VirtuaOSBuilder ./build.sh
OUTPUT_DIR="${OUTPUT_DIR:-${SCRIPT_DIR}}"
OUTPUT_ISO="${OUTPUT_DIR}/VirtuaOS-${VIRTUAOS_VERSION}-amd64.iso"
DOCKER_IMAGE="virtuaos-builder:${VIRTUAOS_VERSION}"
# Build/work directory. live-build assembles the chroot + squashfs here, so it
# must be on a DISK-backed filesystem with several GB free — never a tmpfs (RAM).
# If the user didn't pin BUILD_CONTEXT and /tmp is RAM-backed (common on build
# VMs / small LXC where /tmp is a tmpfs), fall back to a disk-backed .build/ next
# to the source so `sudo ./build.sh --native` just works. Override explicitly:
#   BUILD_CONTEXT=/var/tmp/virtuaos-build sudo ./build.sh --native
if [[ -n "${BUILD_CONTEXT:-}" ]]; then
    :  # honor user override verbatim
elif [[ "$(stat -f -c %T /tmp 2>/dev/null || echo unknown)" =~ ^(tmpfs|ramfs)$ ]]; then
    BUILD_CONTEXT="${SCRIPT_DIR}/.build/virtuaos-build-$$"
else
    BUILD_CONTEXT="/tmp/virtuaos-build-$$"
fi
NATIVE=false
VALIDATE_ONLY=false

# ── AuxiNux kernel repository ────────────────────────────────────────────────
# VirtuaOS ships the AuxiNux hypervisor kernel (kernel-virtua / 7.x-auxinux-virtua)
# instead of the stock Debian kernel. The signing key is required so apt trusts
# https://dep.auxinux.ca/VIRTUA both at build time (chroot install of kernel-virtua)
# and on the installed system (future `apt full-upgrade`).
#
#   AUXINUX_KEYRING_URL  : where to download the ASCII-armored repo key.
#   AUXINUX_KEYRING_FILE : local armored key to use instead of downloading
#                          (use this while the public repo is not yet published,
#                           e.g. the file produced by build-depot-kernel.sh:
#                           /var/www/dep.auxinux.ca/VIRTUA/virtua-archive-keyring.asc).
AUXINUX_KEYRING_URL="${AUXINUX_KEYRING_URL:-https://dep.auxinux.ca/VIRTUA/virtua-archive-keyring.asc}"
AUXINUX_KEYRING_FILE="${AUXINUX_KEYRING_FILE:-}"

# ── Colors ────────────────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
CYAN='\033[0;36m'; BOLD='\033[1m'; NC='\033[0m'
info()    { echo -e "${CYAN}[INFO]${NC}  $*"; }
ok()      { echo -e "${GREEN}[OK]${NC}    $*"; }
warn()    { echo -e "${YELLOW}[WARN]${NC}  $*"; }
err()     { echo -e "${RED}[ERR]${NC}   $*"; exit 1; }
step()    { echo -e "\n${BOLD}${CYAN}══ $*${NC}"; }

# Fail fast on a broken/incomplete checkout (missing sibling Virtua repo,
# missing installer, wrong kernel config, hook syntax errors) rather than
# discovering it 20 minutes into a live-build run. Used standalone by
# `--validate` and as the first step of every real build.
validate_project_inputs() {
    step "Validating build inputs"

    local installer="${SCRIPT_DIR}/livecd/includes.chroot/usr/local/sbin/virtuaos-install"
    local config="${SCRIPT_DIR}/livecd/config.sh"
    local kernel_list="${SCRIPT_DIR}/livecd/archives/auxinux-kernel.list.chroot"

    [[ -f "${config}" ]] || err "Missing live-build config: ${config}"
    [[ -f "${installer}" ]] || err "Missing installer: ${installer}"
    [[ -x "${installer}" ]] || err "Installer is not executable: ${installer}"
    [[ -f "${kernel_list}" ]] || err "Missing AuxiNux kernel apt source: ${kernel_list}"

    # The panel is built from the npm monorepo at the repo root (PROJECT_ROOT =
    # ../Virtua. If only VirtuaOS/ was copied to a build box,
    # PROJECT_ROOT points at an unrelated dir (e.g. $HOME) and hook 0400 dies far
    # later with a cryptic "ENOENT /opt/auxinux-virtua/package.json". Fail now.
    local _missing=()
    for _need in package.json apps packages lang tsconfig.base.json INSTALL/vdm-install.sh INSTALL/vdm-ha-agent apps/vdm apps/vdm-ui; do
        [[ -e "${PROJECT_ROOT}/${_need}" ]] || _missing+=("${_need}")
    done
    if [[ ${#_missing[@]} -gt 0 ]]; then
        err "Monorepo source not found at PROJECT_ROOT=${PROJECT_ROOT}
Missing: ${_missing[*]}
build.sh bakes the Virtua panel from the sibling Virtua project.
Copy/clone both Virtua/ and VirtuaOS/ to the build box, then re-run from inside VirtuaOS/."
    fi

    # The kernel must come from kernel-virtua, never the stock Debian image.
    if grep -Eq '^[[:space:]]*linux-image-amd64[[:space:]]*$' \
        "${SCRIPT_DIR}/livecd/package-lists/00-base.list.chroot"; then
        err "linux-image-amd64 is still listed in 00-base.list.chroot — VirtuaOS must ship kernel-virtua only"
    fi
    grep -Eq '^[[:space:]]*--linux-flavours[[:space:]]+"virtua"' "${config}" \
        || err "config.sh must set --linux-flavours \"virtua\" to install the AuxiNux kernel"

    bash -n "${installer}" || err "Installer syntax validation failed"
    bash -n "${config}" || err "Live-build config syntax validation failed"

    local hook
    while IFS= read -r hook; do
        bash -n "${hook}" || err "Hook syntax validation failed: ${hook}"
    done < <(find "${SCRIPT_DIR}/livecd/hooks" -type f -name "*.hook.*" | sort)

    ok "Build inputs validated"
}

# Post-build sanity check on a finished ISO: extracts its squashfs and
# confirms the embedded installer matches the current source (catches a
# stale/cached chroot), and that bash/grub-install/the dynamic loader and the
# AuxiNux kernel are actually present in the live filesystem. Called at the
# end of `build_native`; the Docker path runs the equivalent checks inside
# the container (see inner-build.sh below).
validate_iso_contents() {
    local iso="$1"

    step "Validating ISO installer payload"

    command -v xorriso >/dev/null 2>&1 || {
        warn "xorriso is not available; skipping ISO payload validation"
        return 0
    }
    command -v unsquashfs >/dev/null 2>&1 || {
        warn "unsquashfs is not available; skipping ISO payload validation"
        return 0
    }

    local check_dir filesystem listing
    check_dir="$(mktemp -d)"
    filesystem="${check_dir}/filesystem.squashfs"
    listing="${check_dir}/filesystem.txt"

    if ! xorriso -osirrox on -indev "${iso}" -extract /live/filesystem.squashfs "${filesystem}" >/dev/null 2>&1; then
        rm -rf "${check_dir}"
        err "Unable to extract /live/filesystem.squashfs from ${iso}"
    fi

    if ! unsquashfs -cat "${filesystem}" usr/local/sbin/virtuaos-install | grep -q "validate_target_runtime"; then
        rm -rf "${check_dir}"
        err "Generated ISO does not contain the runtime-safe VirtuaOS installer"
    fi

    # Freshness guard: the installer embedded in the ISO must be byte-identical to
    # the current source. Catches a build that silently shipped an old installer
    # (includes.chroot not applied, cached chroot, etc.) — the exact failure mode
    # behind "I rebuilt but the installer UI is unchanged".
    local src_installer source_sum embedded_sum
    src_installer="${SCRIPT_DIR}/livecd/includes.chroot/usr/local/sbin/virtuaos-install"
    source_sum="$(sha256sum "${src_installer}" | awk '{print $1}')"
    embedded_sum="$(unsquashfs -cat "${filesystem}" usr/local/sbin/virtuaos-install | sha256sum | awk '{print $1}')"
    if [[ "${source_sum}" != "${embedded_sum}" ]]; then
        rm -rf "${check_dir}"
        err "Embedded installer does NOT match current source (stale ISO / includes.chroot not applied).
  source   : ${source_sum}
  embedded : ${embedded_sum}"
    fi

    unsquashfs -ll "${filesystem}" \
        bin/bash usr/bin/bash usr/bin/env usr/sbin/grub-install \
        lib64 usr/lib64 usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2 \
        > "${listing}" 2>/dev/null || true

    grep -Eq "squashfs-root/(bin/bash|usr/bin/bash)" "${listing}" || {
        rm -rf "${check_dir}"
        err "Generated ISO live filesystem is missing bash"
    }
    grep -q "squashfs-root/usr/bin/env" "${listing}" || {
        rm -rf "${check_dir}"
        err "Generated ISO live filesystem is missing /usr/bin/env"
    }
    grep -q "squashfs-root/usr/sbin/grub-install" "${listing}" || {
        rm -rf "${check_dir}"
        err "Generated ISO live filesystem is missing grub-install"
    }
    grep -q "ld-linux-x86-64.so.2" "${listing}" || {
        rm -rf "${check_dir}"
        err "Generated ISO live filesystem is missing the amd64 dynamic loader"
    }

    # The whole point of VirtuaOS: the AuxiNux hypervisor kernel must be the one
    # embedded — not the stock Debian linux-image-amd64.
    local kernel_listing="${check_dir}/kernel.txt"
    unsquashfs -ll "${filesystem}" boot > "${kernel_listing}" 2>/dev/null || true
    grep -Eq "boot/vmlinuz-.*auxinux-virtua" "${kernel_listing}" || {
        rm -rf "${check_dir}"
        err "Generated ISO does not embed the AuxiNux kernel (no boot/vmlinuz-*-auxinux-virtua-* in live filesystem)"
    }

    rm -rf "${check_dir}"
    ok "ISO installer payload validated (AuxiNux kernel present)"
}

# ── Parse args ────────────────────────────────────────────────────────────────
for arg in "$@"; do
    case "$arg" in
        --native) NATIVE=true ;;
        --validate) VALIDATE_ONLY=true ;;
        -h|--help)
            cat <<EOF
VirtuaOS ${VIRTUAOS_VERSION} ISO Builder

Usage:
  ./build.sh           Build ISO using Docker (macOS or any OS with Docker)
  ./build.sh --native  Build ISO natively (requires Debian/Ubuntu + live-build)
  ./build.sh --validate
                       Validate paths and build inputs without building an ISO

Output: ${OUTPUT_ISO}

Environment:
  VIRTUA_SOURCE_DIR=/path/to/Virtua
      Path to the separately cloned Virtua controller repository.
EOF
            exit 0 ;;
        *) err "Unknown argument: $arg. Use --help for usage." ;;
    esac
done

# ── Detect platform ───────────────────────────────────────────────────────────
detect_platform() {
    local os
    os="$(uname -s)"
    case "$os" in
        Darwin) echo "macos" ;;
        Linux)
            if [[ -f /etc/os-release ]]; then
                . /etc/os-release
                echo "linux-${ID:-unknown}"
            else
                echo "linux-unknown"
            fi ;;
        *) echo "unsupported" ;;
    esac
}

PLATFORM="$(detect_platform)"

if [[ "$VALIDATE_ONLY" == "true" ]]; then
    validate_project_inputs
    ok "VirtuaOS source layout is valid"
    exit 0
fi

# ── ─────────────────────────────────────────────────────────────────────────
#   SHARED: prepare build context (copies project source into temp dir)
# ─────────────────────────────────────────────────────────────────────────────
# ── ─────────────────────────────────────────────────────────────────────────
#   Provision the AuxiNux kernel repo signing key into the build context.
#   The same armored key is placed in two spots of the live-build config:
#     1. archives/auxinux-kernel.key.chroot      → trusted during the chroot
#        build so `kernel-virtua` installs from dep.auxinux.ca (then removed
#        from the final image by live-build).
#     2. includes.chroot/usr/share/keyrings/virtua-archive-keyring.asc → persists
#        on the installed system for `apt full-upgrade` of the kernel later.
#   apt accepts an ASCII-armored keyring directly, so no gpg/dearmor is needed.
# ─────────────────────────────────────────────────────────────────────────────
provision_kernel_repo_key() {
    step "Provisioning AuxiNux kernel repository key"

    local livecd_dir="${BUILD_CONTEXT}/livecd"
    local armored="${BUILD_CONTEXT}/virtua-archive-keyring.asc"

    if [[ -n "${AUXINUX_KEYRING_FILE}" ]]; then
        [[ -f "${AUXINUX_KEYRING_FILE}" ]] \
            || err "AUXINUX_KEYRING_FILE set but not found: ${AUXINUX_KEYRING_FILE}"
        info "Using local kernel repo key: ${AUXINUX_KEYRING_FILE}"
        cp "${AUXINUX_KEYRING_FILE}" "${armored}"
    else
        info "Downloading kernel repo key: ${AUXINUX_KEYRING_URL}"
        if command -v curl >/dev/null 2>&1; then
            curl -fsSL "${AUXINUX_KEYRING_URL}" -o "${armored}" || \
                err "Could not download kernel repo key from ${AUXINUX_KEYRING_URL}.
Is dep.auxinux.ca/VIRTUA published yet? If you are still building/publishing the
kernel, re-run with AUXINUX_KEYRING_FILE=/path/to/virtua-archive-keyring.asc"
        elif command -v wget >/dev/null 2>&1; then
            wget -qO "${armored}" "${AUXINUX_KEYRING_URL}" || \
                err "Could not download kernel repo key from ${AUXINUX_KEYRING_URL} (see AUXINUX_KEYRING_FILE)"
        else
            err "Neither curl nor wget available to fetch ${AUXINUX_KEYRING_URL}"
        fi
    fi

    grep -q "PGP PUBLIC KEY BLOCK" "${armored}" \
        || err "Kernel repo key is not an ASCII-armored PGP public key: ${armored}"

    # 1. Build-time trust (live-build archives mechanism).
    # 2. Persistent keyring referenced by the installed-system apt source.
    # (Use mkdir+cp, not `install -D` — the latter is GNU-only and this runs on
    #  the host, which may be macOS/BSD.)
    local build_key="${livecd_dir}/archives/auxinux-kernel.key.chroot"
    local persist_key="${livecd_dir}/includes.chroot/usr/share/keyrings/virtua-archive-keyring.asc"
    mkdir -p "$(dirname "${build_key}")" "$(dirname "${persist_key}")"
    cp "${armored}" "${build_key}"   && chmod 0644 "${build_key}"
    cp "${armored}" "${persist_key}" && chmod 0644 "${persist_key}"

    rm -f "${armored}"
    ok "Kernel repo key provisioned (build-time + persistent)"
}

# Stage locally-built .deb packages (auxinux-virtua, virtuaos-cli) into the
# live-build packages.chroot dir so live-build installs them in the chroot and
# dpkg REGISTERS them. This is the "hybrid" model: the panel is still baked and
# built offline by hook 0400 (so the first boot needs no network), but because
# dpkg now knows the installed version, `virtuaos update` / apt can upgrade it
# later from dep.auxinux.ca. The .deb's setup unit ships DORMANT — its postinst
# can't enable it inside a chroot (no running systemd) — so nothing re-provisions
# at first boot; an apt upgrade on the running system is what triggers install.sh.
LOCAL_DEB_DIR="${LOCAL_DEB_DIR:-${PROJECT_ROOT}/packaging/deb/out}"
stage_local_debs() {
    local dest="${BUILD_CONTEXT}/livecd/packages.chroot"
    local found=0 deb
    local virtua_version cli_version
    virtua_version="$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' "${PROJECT_ROOT}/package.json" | head -1)"
    cli_version="$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "${SCRIPT_DIR}/virtuaos-cli/Cargo.toml" | head -1)"
    if [[ -d "${LOCAL_DEB_DIR}" ]]; then
        mkdir -p "${dest}"
        for deb in \
            "${LOCAL_DEB_DIR}/auxinux-virtua_${virtua_version}_amd64.deb" \
            "${LOCAL_DEB_DIR}/virtuaos-cli_${cli_version}_amd64.deb"; do
            if [[ -f "${deb}" ]]; then
                cp "${deb}" "${dest}/"
                info "  staged $(basename "${deb}") → packages.chroot/"
                found=1
            else
                warn "Current package is missing and will not be registered: $(basename "${deb}")"
            fi
        done
    fi
    if [[ "${found}" -eq 1 ]]; then
        ok "Local .deb(s) staged — dpkg will register them in the image (hybrid mode)"
    else
        warn "No local .deb in ${LOCAL_DEB_DIR}; the ISO will bake Virtua WITHOUT dpkg registration.
Build it first so 'virtuaos update' can manage it:  (cd packaging/deb && ./build-deb.sh)"
    fi
}

# Build the Rust TUI installer front-end (virtuaos-installer) for amd64 and stage
# it into the live image at /usr/local/sbin/virtuaos-installer. Native cargo on
# Linux/amd64 (installs cargo if missing), else Docker (rust:slim, linux/amd64).
# NON-FATAL: if it can't be built, the ISO still ships and the live env falls
# back to the proven bash wizard (see hook 0150 .bash_profile).
provision_installer_binary() {
    step "Building Rust installer front-end (virtuaos-installer)"
    local crate="${SCRIPT_DIR}/virtuaos-installer"
    local dest="${BUILD_CONTEXT}/livecd/includes.chroot/usr/local/sbin/virtuaos-installer"
    local built="${crate}/target/release/virtuaos-installer"

    if [[ ! -f "${crate}/Cargo.toml" ]]; then
        warn "virtuaos-installer crate not found at ${crate}; live env will use the bash wizard."
        return 0
    fi
    mkdir -p "$(dirname "${dest}")"
    # NOTE: deliberately do NOT delete an existing ${built} here. If the rebuild
    # below fails (no network, Docker down), a previously-built valid binary is
    # still a better outcome than silently dropping to the bash wizard — which is
    # exactly the "installer is still the full script" regression we hit before.

    if [[ "$(uname -s)" == "Linux" && "$(uname -m)" == "x86_64" ]]; then
        if ! command -v cargo >/dev/null 2>&1; then
            info "Installing cargo for the installer build…"
            apt-get install -y cargo >/dev/null 2>&1 || true
        fi
        if command -v cargo >/dev/null 2>&1; then
            ( cd "${crate}" && cargo build --release ) || warn "native cargo build of installer failed"
        fi
    fi

    if [[ ! -f "${built}" ]] && command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
        info "Building installer in Docker (rust:slim, linux/amd64)…"
        docker run --rm --platform linux/amd64 \
            -v "${SCRIPT_DIR}:/src/virtuaos" -w /src/virtuaos/virtuaos-installer \
            rust:slim bash -c "cargo build --release" \
            || warn "Docker installer build failed"
    fi

    # Only stage a Linux x86-64 ELF. A host-native binary (e.g. a Mach-O left in
    # target/release from a local `cargo build` on macOS) must never be shipped —
    # it would not run in the live image and the launcher would fall back to bash.
    if [[ -f "${built}" ]] && file "${built}" 2>/dev/null | grep -q 'ELF.*x86-64'; then
        cp "${built}" "${dest}"
        chmod 0755 "${dest}"
        ok "Installer front-end staged → /usr/local/sbin/virtuaos-installer ($(file -b "${built}" 2>/dev/null | cut -d, -f1))"
    elif [[ -f "${built}" ]]; then
        warn "Built installer is not a Linux x86-64 ELF ($(file -b "${built}" 2>/dev/null)) — NOT staging; live env will use the bash wizard."
    else
        warn "virtuaos-installer not built — live env will fall back to the bash wizard."
    fi
}

# Pull the LATEST published Virtua from the apt repo (dep.auxinux.ca/VIRTUA,
# suite "virtua") AT BUILD TIME — exactly the same model as the kernel. The
# published auxinux-virtua .deb bundles the full monorepo source under
# /opt/auxinux-virtua, so we can keep the offline-first-boot design AND always
# ship the latest released version:
#   • the .deb(s) are staged into packages.chroot   → dpkg registers the version
#     (so `virtuaos update` / apt can upgrade later)
#   • the source payload is extracted into ${BUILD_CONTEXT}/virtua-src → hook
#     0400 compiles it OFFLINE in the chroot (first boot needs no network).
# Returns non-zero (→ caller falls back to the local monorepo) if the repo is
# unreachable or VIRTUA_FROM_REPO=false.
VIRTUA_FROM_REPO="${VIRTUA_FROM_REPO:-true}"
VIRTUA_REPO_BASE="${VIRTUA_REPO_BASE:-https://dep.auxinux.ca/VIRTUA}"
VIRTUA_REPO_SUITE="${VIRTUA_REPO_SUITE:-virtua}"
provision_virtua_from_repo() {
    [[ "${VIRTUA_FROM_REPO}" == "true" ]] || return 1
    command -v curl >/dev/null 2>&1 || { warn "curl missing — cannot fetch Virtua from repo"; return 1; }
    command -v ar   >/dev/null 2>&1 || { warn "ar (binutils) missing — cannot unpack Virtua .deb"; return 1; }
    command -v tar  >/dev/null 2>&1 || { warn "tar missing — cannot unpack Virtua .deb"; return 1; }

    step "Fetching latest Virtua from repo (${VIRTUA_REPO_BASE} suite ${VIRTUA_REPO_SUITE})"
    local tmp pkgidx
    tmp="$(mktemp -d)"
    pkgidx="${VIRTUA_REPO_BASE}/dists/${VIRTUA_REPO_SUITE}/main/binary-amd64/Packages"
    if ! curl -fsSL "${pkgidx}" -o "${tmp}/Packages"; then
        warn "Could not fetch ${pkgidx} (offline?) — will use the local source instead"
        rm -rf "${tmp}"; return 1
    fi

    # Read a field for a given package from the stanza-structured Packages index.
    _deb_field() {  # $1=package $2=field
        awk -v pkg="$1" -v fld="$2" '
            /^Package:/ { p=$2 }
            $0 ~ "^"fld":" { if (p==pkg) { sub("^"fld":[ \t]*",""); print; exit } }
        ' "${tmp}/Packages"
    }

    local vfile vver cfile
    vfile="$(_deb_field auxinux-virtua Filename)"
    vver="$(_deb_field auxinux-virtua Version)"
    cfile="$(_deb_field virtuaos-cli  Filename)"
    if [[ -z "${vfile}" ]]; then
        warn "auxinux-virtua not found in the repo index — using the local source instead"
        rm -rf "${tmp}"; return 1
    fi

    local dest_pkgs="${BUILD_CONTEXT}/livecd/packages.chroot"
    mkdir -p "${dest_pkgs}"

    local vdeb="${tmp}/$(basename "${vfile}")"
    if ! curl -fsSL "${VIRTUA_REPO_BASE}/${vfile}" -o "${vdeb}"; then
        warn "Failed to download ${vfile} — using the local source instead"
        rm -rf "${tmp}"; return 1
    fi
    cp "${vdeb}" "${dest_pkgs}/"
    info "  staged $(basename "${vdeb}") → packages.chroot/ (dpkg registration)"

    if [[ -n "${cfile}" ]]; then
        local cdeb="${tmp}/$(basename "${cfile}")"
        if curl -fsSL "${VIRTUA_REPO_BASE}/${cfile}" -o "${cdeb}"; then
            cp "${cdeb}" "${dest_pkgs}/"
            info "  staged $(basename "${cdeb}") → packages.chroot/"
        fi
    fi

    info "Extracting Virtua source from the published .deb…"
    local x="${tmp}/x"; mkdir -p "${x}"
    ( cd "${x}" && ar x "${vdeb}" ) || { warn "ar failed on $(basename "${vdeb}")"; rm -rf "${tmp}"; return 1; }
    local data
    data="$(ls "${x}"/data.tar.* 2>/dev/null | head -1)"
    [[ -n "${data}" ]] || { warn "no data.tar in $(basename "${vdeb}")"; rm -rf "${tmp}"; return 1; }
    tar -xf "${data}" -C "${tmp}" ./opt/auxinux-virtua 2>/dev/null || tar -xf "${data}" -C "${tmp}"
    if [[ ! -f "${tmp}/opt/auxinux-virtua/package.json" ]]; then
        warn "extracted .deb payload has no /opt/auxinux-virtua/package.json — using the local source"
        rm -rf "${tmp}"; return 1
    fi
    rm -rf "${BUILD_CONTEXT}/virtua-src"
    mkdir -p "${BUILD_CONTEXT}/virtua-src"
    rsync -a "${tmp}/opt/auxinux-virtua/" "${BUILD_CONTEXT}/virtua-src/"

    rm -rf "${tmp}"
    ok "Virtua ${vver:-(published)} sourced from repo — baked offline + dpkg-registered"
    return 0
}

prepare_build_context() {
    step "Preparing build context"
    validate_project_inputs
    rm -rf "${BUILD_CONTEXT}"
    mkdir -p "${BUILD_CONTEXT}/livecd"
    mkdir -p "${BUILD_CONTEXT}/virtua-src"

    info "Copying VirtuaOS livecd config…"
    rsync -a "${SCRIPT_DIR}/livecd/" "${BUILD_CONTEXT}/livecd/"

    provision_kernel_repo_key
    provision_installer_binary

    # Prefer the LATEST PUBLISHED Virtua from the apt repo (like the kernel). On
    # failure (offline / VIRTUA_FROM_REPO=false), fall back to baking the LOCAL
    # monorepo source + the locally-built .deb.
    if ! provision_virtua_from_repo; then
        warn "Using LOCAL Virtua source/.deb (repo unavailable or VIRTUA_FROM_REPO=false)"
        stage_local_debs

        info "Copying Virtua source (excluding build artifacts)…"
        # Only the npm monorepo (workspaces = packages/* + apps/*) and the root
        # manifests are needed to build the panel in the chroot. Everything else is
        # excluded — most importantly Client_Desktop/ (its src-tauri/target/ alone is
        # ~11 GB of Rust build artifacts). Shipping it would bloat the build context
        # to ~16 GB and, on a memory-constrained host (e.g. a 4 GB LXC with a tmpfs
        # /tmp), exhaust RAM and get the build OOM-killed ("Killed" on ldconfig).
        rsync -a \
            --exclude='node_modules' --exclude='dist' --exclude='.git' \
            --exclude='OS' --exclude='*.iso' --exclude='.DS_Store' \
            --exclude='Client_Desktop' --exclude='packaging' \
            --exclude='img' --exclude='docs' --exclude='.claude' \
            --exclude='*.pdf' \
            "${PROJECT_ROOT}/" "${BUILD_CONTEXT}/virtua-src/"
    fi

    ok "Build context ready: ${BUILD_CONTEXT}"
}

# ─────────────────────────────────────────────────────────────────────────────
#   DOCKER BUILD (macOS or any OS with Docker)
# ─────────────────────────────────────────────────────────────────────────────
# Build the ISO inside a throwaway Debian container (linux/amd64, emulated on
# Apple Silicon). Generates a Dockerfile + inner-build.sh into BUILD_CONTEXT,
# builds the image, then runs it with --privileged (live-build needs to
# mount/chroot) and an /output bind mount to collect the finished ISO.
build_with_docker() {
    step "Building VirtuaOS ${VIRTUAOS_VERSION} via Docker (x86_64 emulation)"

    command -v docker >/dev/null 2>&1 || \
        err "Docker not found. Install Docker Desktop: https://www.docker.com/products/docker-desktop/"

    # Check Docker is running
    docker info >/dev/null 2>&1 || err "Docker daemon is not running. Start Docker Desktop first."

    prepare_build_context

    info "Writing Dockerfile…"
    cat > "${BUILD_CONTEXT}/Dockerfile" <<'DOCKERFILE'
FROM debian:trixie-slim

ENV DEBIAN_FRONTEND=noninteractive
ENV LC_ALL=C.UTF-8
ENV LANG=C.UTF-8

# Install build tools
RUN apt-get update -qq && \
    apt-get install -y --no-install-recommends \
        live-build xorriso isolinux syslinux-common \
        debootstrap squashfs-tools genisoimage \
        rsync wget curl ca-certificates gpg \
        git make && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Copy livecd config
COPY livecd/ ./config/

# Copy Virtua source into live-build chroot includes
# (live-build picks up config/includes.chroot/**  and overlays it into the chroot)
RUN mkdir -p ./config/includes.chroot/opt/auxinux-virtua
COPY virtua-src/ ./config/includes.chroot/opt/auxinux-virtua/

# Fix permissions on all hooks (chroot + binary) and included scripts
RUN find ./config/hooks -name "*.hook.chroot" -exec chmod +x {} \; && \
    find ./config/hooks -name "*.hook.binary" -exec chmod +x {} \; && \
    find ./config/includes.chroot/usr/local/bin -type f -exec chmod +x {} \; 2>/dev/null || true && \
    find ./config/includes.chroot/etc/profile.d -type f -exec chmod +x {} \; 2>/dev/null || true && \
    apt-get install -y --no-install-recommends perl 2>/dev/null || true

# Copy inner build script
COPY inner-build.sh /build/inner-build.sh
RUN chmod +x /build/inner-build.sh

CMD ["/build/inner-build.sh"]
DOCKERFILE

    info "Writing inner build script…"
    cat > "${BUILD_CONTEXT}/inner-build.sh" <<INNERSH
#!/usr/bin/env bash
set -euo pipefail
cd /build

# Run lb config
source /build/config/config.sh

# Run lb build
lb build 2>&1 | tee /build/build.log

echo "--- Listing generated files ---"
find /build -maxdepth 1 \( -name "*.iso" -o -name "*.img" -o -name "*.hybrid.iso" \) 2>/dev/null || true

# Find ISO — use find to catch any naming convention live-build may use
ISO=\$(find /build -maxdepth 1 \( -name "*.iso" -o -name "*.hybrid.iso" \) 2>/dev/null | head -1 || true)
[[ -n "\$ISO" ]] || {
    echo "Build failed — no ISO found in /build"
    echo "Contents of /build:"
    ls -lh /build/ 2>/dev/null || true
    exit 1
}

echo "--- Validating ISO installer payload ---"
CHECK_DIR=\$(mktemp -d)
FILESYSTEM="\${CHECK_DIR}/filesystem.squashfs"
LISTING="\${CHECK_DIR}/filesystem.txt"

if ! xorriso -osirrox on -indev "\$ISO" -extract /live/filesystem.squashfs "\${FILESYSTEM}" >/dev/null 2>&1; then
    echo "Unable to extract /live/filesystem.squashfs from generated ISO"
    rm -rf "\${CHECK_DIR}"
    exit 1
fi

if ! unsquashfs -cat "\${FILESYSTEM}" usr/local/sbin/virtuaos-install | grep -q "validate_target_runtime"; then
    echo "Generated ISO does not contain the runtime-safe VirtuaOS installer"
    rm -rf "\${CHECK_DIR}"
    exit 1
fi

# Freshness guard: embedded installer must be byte-identical to the source that
# was COPYd into this image. Catches a build that shipped a stale installer
# (the "I rebuilt but the UI is unchanged" failure mode).
SRC_INSTALLER=/build/config/includes.chroot/usr/local/sbin/virtuaos-install
SOURCE_SUM=\$(sha256sum "\${SRC_INSTALLER}" | awk '{print \$1}')
EMBEDDED_SUM=\$(unsquashfs -cat "\${FILESYSTEM}" usr/local/sbin/virtuaos-install | sha256sum | awk '{print \$1}')
if [[ "\${SOURCE_SUM}" != "\${EMBEDDED_SUM}" ]]; then
    echo "Embedded installer does NOT match source (stale ISO / includes.chroot not applied)"
    echo "  source:   \${SOURCE_SUM}"
    echo "  embedded: \${EMBEDDED_SUM}"
    rm -rf "\${CHECK_DIR}"
    exit 1
fi

unsquashfs -ll "\${FILESYSTEM}" \
    bin/bash usr/bin/bash usr/bin/env usr/sbin/grub-install \
    lib64 usr/lib64 usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2 \
    > "\${LISTING}" 2>/dev/null || true

grep -Eq "squashfs-root/(bin/bash|usr/bin/bash)" "\${LISTING}" || {
    echo "Generated ISO live filesystem is missing bash"
    rm -rf "\${CHECK_DIR}"
    exit 1
}
grep -q "squashfs-root/usr/bin/env" "\${LISTING}" || {
    echo "Generated ISO live filesystem is missing /usr/bin/env"
    rm -rf "\${CHECK_DIR}"
    exit 1
}
grep -q "squashfs-root/usr/sbin/grub-install" "\${LISTING}" || {
    echo "Generated ISO live filesystem is missing grub-install"
    rm -rf "\${CHECK_DIR}"
    exit 1
}
grep -q "ld-linux-x86-64.so.2" "\${LISTING}" || {
    echo "Generated ISO live filesystem is missing the amd64 dynamic loader"
    rm -rf "\${CHECK_DIR}"
    exit 1
}

# The AuxiNux hypervisor kernel must be embedded — not the stock Debian kernel.
unsquashfs -ll "\${FILESYSTEM}" boot > "\${CHECK_DIR}/kernel.txt" 2>/dev/null || true
grep -Eq "boot/vmlinuz-.*auxinux-virtua" "\${CHECK_DIR}/kernel.txt" || {
    echo "Generated ISO does not embed the AuxiNux kernel (no boot/vmlinuz-*-auxinux-virtua-*)"
    rm -rf "\${CHECK_DIR}"
    exit 1
}

rm -rf "\${CHECK_DIR}"
echo "--- ISO installer payload validated (AuxiNux kernel present) ---"

# Ensure output dir exists (fallback if volume not mounted)
mkdir -p /output
cp "\$ISO" /output/VirtuaOS-${VIRTUAOS_VERSION}-amd64.iso
echo "ISO ready: /output/VirtuaOS-${VIRTUAOS_VERSION}-amd64.iso  (\$(du -sh "\$ISO" | cut -f1))"
INNERSH
    chmod +x "${BUILD_CONTEXT}/inner-build.sh"

    info "Building Docker image (${DOCKER_IMAGE})…"
    docker build \
        --platform linux/amd64 \
        -t "${DOCKER_IMAGE}" \
        "${BUILD_CONTEXT}"

    info "Running ISO build inside container (this may take 30-60 minutes on Apple Silicon)…"
    mkdir -p "${OUTPUT_DIR}/output"
    docker run --rm \
        --platform linux/amd64 \
        --privileged \
        -v "${OUTPUT_DIR}/output:/output" \
        "${DOCKER_IMAGE}"

    # Move ISO to final location
    if [[ -f "${OUTPUT_DIR}/output/VirtuaOS-${VIRTUAOS_VERSION}-amd64.iso" ]]; then
        mv "${OUTPUT_DIR}/output/VirtuaOS-${VIRTUAOS_VERSION}-amd64.iso" "${OUTPUT_ISO}"
        rmdir "${OUTPUT_DIR}/output" 2>/dev/null || true
    fi

    # Cleanup
    rm -rf "${BUILD_CONTEXT}"
    docker rmi "${DOCKER_IMAGE}" >/dev/null 2>&1 || true
}

# ─────────────────────────────────────────────────────────────────────────────
#   NATIVE BUILD (Debian/Ubuntu host only)
# ─────────────────────────────────────────────────────────────────────────────
# Build the ISO directly on a Debian/Ubuntu host via live-build (`lb build`).
# Requires root (live-build chroots/mounts need it) and a disk-backed work
# directory with several GB free — most of this function is preflight guards
# against the two failure modes that used to bite here: a tmpfs work dir
# (OOM-killed) and a PATH missing /usr/sbin under `sudo` (debootstrap "not found").
build_native() {
    step "Building VirtuaOS ${VIRTUAOS_VERSION} natively"

    [[ $EUID -eq 0 ]] || err "Native build requires root: sudo ./build.sh --native"

    # Guard against building on a tmpfs (RAM-backed) work dir. live-build writes a
    # multi-GB chroot + squashfs under BUILD_CONTEXT; if that lands on a tmpfs
    # /tmp (common on small LXC containers), it fills RAM and the build is
    # OOM-killed ("Killed" on a libc-bin/ldconfig trigger) long before the ISO.
    mkdir -p "${BUILD_CONTEXT}"
    local _wd_fstype
    _wd_fstype="$(stat -f -c %T "${BUILD_CONTEXT}" 2>/dev/null || echo unknown)"
    if [[ "${_wd_fstype}" == "tmpfs" || "${_wd_fstype}" == "ramfs" ]]; then
        err "Build dir ${BUILD_CONTEXT} is on a ${_wd_fstype} (RAM) filesystem.
This will exhaust memory and the build will be OOM-killed. Point BUILD_CONTEXT at a
disk-backed path with several GB free, e.g.:
    sudo BUILD_CONTEXT=${PWD}/.build ./build.sh --native
    (or any dir on your 50 GB disk, NOT under a tmpfs /tmp)"
    fi

    # Disk-space preflight: live-build needs several GB (chroot + squashfs + ISO).
    # Failing here with a clear message beats a confusing ENOSPC mid-chroot
    # ("0 B available" / "npm: command not found" once apt can't unpack).
    local _avail_kb
    _avail_kb="$(df -Pk "${BUILD_CONTEXT}" 2>/dev/null | awk 'NR==2{print $4}')"
    if [[ -n "${_avail_kb}" && "${_avail_kb}" -lt 8388608 ]]; then
        err "Only $(( _avail_kb / 1024 )) MB free on the filesystem holding ${BUILD_CONTEXT}.
The ISO build needs ~8 GB. Free space (old builds, docker images) or point
BUILD_CONTEXT at a roomier disk:
    sudo BUILD_CONTEXT=/path/with/space/.build ./build.sh --native"
    fi

    # live-build invokes debootstrap, mksquashfs, xorrisofs, etc., which all
    # live under /usr/sbin and /sbin. Some shells (especially when bash is
    # launched non-interactively, or via `sudo` without `-i`) inherit a PATH
    # that omits these directories, leading to the confusing
    #   "E: debootstrap - command not found"
    # even though the package is installed. Force a sane root PATH up-front.
    export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:${PATH:-}"

    # Install live-build and validation tooling if anything is missing.
    if ! command -v lb >/dev/null 2>&1 || \
       ! command -v xorriso >/dev/null 2>&1 || \
       ! command -v unsquashfs >/dev/null 2>&1 || \
       ! command -v rsync >/dev/null 2>&1 || \
       ! command -v debootstrap >/dev/null 2>&1; then
        info "Installing native build dependencies…"
        apt-get update
        apt-get install -y \
            live-build xorriso isolinux syslinux-common debootstrap squashfs-tools \
            genisoimage rsync wget curl ca-certificates gpg git make
    fi

    command -v debootstrap >/dev/null 2>&1 \
        || err "debootstrap is installed but not on PATH (looked in: ${PATH}). Run: which debootstrap"

    prepare_build_context

    WORKDIR="${BUILD_CONTEXT}/workspace"
    mkdir -p "${WORKDIR}"
    cd "${WORKDIR}"

    # Copy config
    cp -r "${BUILD_CONTEXT}/livecd/." ./config/
    find ./config/hooks -name "*.hook.chroot" -exec chmod +x {} \;
    find ./config/hooks -name "*.hook.binary" -exec chmod +x {} \;
    find ./config/includes.chroot/usr/local/bin -type f -exec chmod +x {} \; 2>/dev/null || true
    find ./config/includes.chroot/etc/profile.d -type f -exec chmod +x {} \; 2>/dev/null || true

    # Copy Virtua source into live-build chroot includes
    # (live-build overlays config/includes.chroot/** into the chroot before hooks run)
    mkdir -p ./config/includes.chroot/opt/auxinux-virtua
    rsync -a "${BUILD_CONTEXT}/virtua-src/" ./config/includes.chroot/opt/auxinux-virtua/

    # Run lb config
    source ./config/config.sh

    # Run lb build
    lb build 2>&1 | tee /tmp/virtuaos-build.log

    ISO="$(ls "${WORKDIR}"/VirtuaOS-*.hybrid.iso "${WORKDIR}"/live-image*.iso 2>/dev/null | head -1 || true)"
    [[ -n "$ISO" ]] || err "Build failed — check /tmp/virtuaos-build.log"
    cp "$ISO" "${OUTPUT_ISO}"
    validate_iso_contents "${OUTPUT_ISO}"

    rm -rf "${BUILD_CONTEXT}"
}

# ─────────────────────────────────────────────────────────────────────────────
#   MAIN
# ─────────────────────────────────────────────────────────────────────────────
echo ""
echo -e "${BOLD}╔════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║      VirtuaOS ${VIRTUAOS_VERSION} — ISO Builder       ║${NC}"
echo -e "${BOLD}║  Debian 13 Trixie • AuxiNux kernel • amd64 ║${NC}"
echo -e "${BOLD}╚════════════════════════════════════════════╝${NC}"
echo ""
info "Platform  : ${PLATFORM}"
info "Output    : ${OUTPUT_ISO}"
info "Source    : ${PROJECT_ROOT}"

# Remove any ISO left by a PREVIOUS build before we start. If this build fails,
# the user must not be left with a stale ISO masquerading as a fresh result —
# that is the classic "I rebuilt but my changes aren't there" trap (they boot
# last build's ISO). A failed build now leaves NO ISO at all.
rm -f "${OUTPUT_ISO}" "${SCRIPT_DIR}/output/VirtuaOS-${VIRTUAOS_VERSION}-amd64.iso" 2>/dev/null || true
[[ -e "${OUTPUT_ISO}" ]] && warn "Could not remove stale ISO ${OUTPUT_ISO} — delete it manually before booting."

if [[ "$NATIVE" == "true" ]]; then
    build_native
else
    build_with_docker
fi

if [[ -f "${OUTPUT_ISO}" ]]; then
    echo ""
    echo -e "${BOLD}${GREEN}╔════════════════════════════════════════════╗${NC}"
    echo -e "${BOLD}${GREEN}║           ISO BUILD SUCCESSFUL ✓           ║${NC}"
    echo -e "${BOLD}${GREEN}╚════════════════════════════════════════════╝${NC}"
    echo ""
    ok "ISO  : ${OUTPUT_ISO}"
    ok "Size : $(du -sh "${OUTPUT_ISO}" | cut -f1)"
    echo ""
    echo "  ── Flash to USB (Linux/macOS) ──────────────────────────────"
    echo "  sudo dd if=\"${OUTPUT_ISO}\" of=/dev/sdX bs=4M status=progress conv=fsync"
    echo "  (or use Balena Etcher / Rufus on Windows)"
    echo ""
    echo "  ── Boot Menu options ───────────────────────────────────────"
    echo "  • Install VirtuaOS        (interactive, you choose disk)"
    echo "  • Install VirtuaOS (auto) (automated, uses first available disk)"
    echo "  • Live shell              (boots without installing)"
    echo ""
    echo "  After install + first boot, Virtua is available at:"
    echo "  http://<SERVER-IP>:8441  —  admin / admin123"
    echo ""
else
    err "Build completed but ISO file was not found at ${OUTPUT_ISO}"
fi
