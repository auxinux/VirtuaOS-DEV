#!/usr/bin/env bash
# =============================================================================
#  VirtuaOS — Dynamic login message
#  Displayed every time a user logs in (SSH or console)
# =============================================================================

# Colors (only if interactive terminal)
if [[ -t 1 ]]; then
    C_BLUE='\033[0;34m'
    C_CYAN='\033[0;36m'
    C_GREEN='\033[0;32m'
    C_RED='\033[0;31m'
    C_YELLOW='\033[1;33m'
    C_BOLD='\033[1m'
    C_DIM='\033[2m'
    C_NC='\033[0m'
else
    C_BLUE=''; C_CYAN=''; C_GREEN=''; C_RED=''
    C_YELLOW=''; C_BOLD=''; C_DIM=''; C_NC=''
fi

# Collect system info
_IP="$(ip route get 8.8.8.8 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src") print $(i+1)}' | head -1 || echo 'N/A')"
_UPTIME="$(uptime -p 2>/dev/null | sed 's/^up //' || echo 'N/A')"
_LOAD="$(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null || echo 'N/A')"
_MEM_TOTAL="$(awk '/MemTotal/ {printf "%.0f", $2/1024}' /proc/meminfo 2>/dev/null || echo '?')"
_MEM_FREE="$(awk '/MemAvailable/ {printf "%.0f", $2/1024}' /proc/meminfo 2>/dev/null || echo '?')"
_DISK="$(df -h / 2>/dev/null | awk 'NR==2 {print $3"/"$2" ("$5")"}' || echo 'N/A')"
_VIRTUA_PORT="8441"

# ── VirtuaOS version (dynamic) ────────────────────────────────────────────────
# Read the release marker written by `virtuaos apply-branding` (kept in sync by
# the virtuaos-cli postinst on every upgrade). Fall back to /etc/os-release so
# the banner never shows a stale hard-coded version.
_VIRTUAOS_VERSION=""
if [[ -f /etc/virtuaos-version ]]; then
    _VIRTUAOS_VERSION="$(tr -d '[:space:]' < /etc/virtuaos-version)"
fi
if [[ -z "${_VIRTUAOS_VERSION}" && -f /etc/os-release ]]; then
    _VIRTUAOS_VERSION="$(sed -n 's/^VERSION_ID="\?\([^"]*\)"\?/\1/p' /etc/os-release | head -1)"
fi
[[ -n "${_VIRTUAOS_VERSION}" ]] || _VIRTUAOS_VERSION="unknown"

# Virtua service status
_API_STATUS="$(systemctl is-active auxinuxvirtual-api 2>/dev/null || echo 'inactive')"
_RUNNER_STATUS="$(systemctl is-active auxinuxvirtual-runner 2>/dev/null || echo 'inactive')"

if [[ "$_API_STATUS" == "active" ]]; then
    _VIRTUA_BADGE="${C_GREEN}● active${C_NC}"
elif [[ "$_API_STATUS" == "activating" ]]; then
    _VIRTUA_BADGE="${C_YELLOW}◌ starting${C_NC}"
else
    _VIRTUA_BADGE="${C_RED}✗ ${_API_STATUS}${C_NC}"
fi

if [[ "$_RUNNER_STATUS" == "active" ]]; then
    _RUNNER_BADGE="${C_GREEN}● active${C_NC}"
else
    _RUNNER_BADGE="${C_RED}✗ ${_RUNNER_STATUS}${C_NC}"
fi

# First-boot check
_FIRSTBOOT_DONE="/etc/virtuaos/.firstboot-done"

echo ""
echo -e "${C_BOLD}${C_CYAN}  ╔════════════════════════════════════════════════╗${C_NC}"
echo -e "${C_BOLD}${C_CYAN}  ║  VirtuaOS ${_VIRTUAOS_VERSION}  •  Debian 13  •  x86_64 ║${C_NC}"
echo -e "${C_BOLD}${C_CYAN}  ╚════════════════════════════════════════════════╝${C_NC}"
echo ""
echo -e "  ${C_DIM}System${C_NC}"
echo -e "    ${C_BOLD}IP Address${C_NC}   : ${_IP}"
echo -e "    ${C_BOLD}Uptime${C_NC}       : ${_UPTIME}"
echo -e "    ${C_BOLD}Load${C_NC}         : ${_LOAD}"
echo -e "    ${C_BOLD}Memory${C_NC}       : ${_MEM_FREE} MB free / ${_MEM_TOTAL} MB total"
echo -e "    ${C_BOLD}Disk (/)${C_NC}     : ${_DISK}"
echo ""
echo -e "  ${C_DIM}Virtua Panel${C_NC}"
echo -e "    ${C_BOLD}API${C_NC}          : $(eval echo -e \"\${_VIRTUA_BADGE}\")"
echo -e "    ${C_BOLD}Runner${C_NC}       : $(eval echo -e \"\${_RUNNER_BADGE}\")"
echo -e "    ${C_BOLD}Web UI${C_NC}       : ${C_CYAN}http://${_IP}:${_VIRTUA_PORT}${C_NC}"
echo -e "    ${C_BOLD}HTTPS${C_NC}        : ${C_CYAN}https://${_IP}${C_NC}  (configure SSL in Settings)"

if [[ ! -f "${_FIRSTBOOT_DONE}" ]]; then
    echo ""
    echo -e "  ${C_YELLOW}⚠  First-boot setup is running (or pending)${C_NC}"
    echo -e "  ${C_YELLOW}   Check progress:  journalctl -fu virtuaos-firstboot${C_NC}"
fi

echo ""
echo -e "  ${C_DIM}Quick commands${C_NC}"
echo -e "    ${C_CYAN}virtuaos-status${C_NC}   — detailed system status"
echo -e "    ${C_CYAN}journalctl -fu auxinuxvirtual-api${C_NC}   — Virtua API logs"
echo -e "    ${C_CYAN}journalctl -fu auxinuxvirtual-runner${C_NC} — Runner logs"
echo ""
