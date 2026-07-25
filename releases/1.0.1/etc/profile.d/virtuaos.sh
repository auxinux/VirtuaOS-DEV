#!/usr/bin/env bash
# VirtuaOS 1.0.1 dynamic login message

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

_IP="$(ip route get 8.8.8.8 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src") print $(i+1)}' | head -1 || echo 'N/A')"
_UPTIME="$(uptime -p 2>/dev/null | sed 's/^up //' || echo 'N/A')"
_LOAD="$(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null || echo 'N/A')"
_MEM_TOTAL="$(awk '/MemTotal/ {printf "%.0f", $2/1024}' /proc/meminfo 2>/dev/null || echo '?')"
_MEM_FREE="$(awk '/MemAvailable/ {printf "%.0f", $2/1024}' /proc/meminfo 2>/dev/null || echo '?')"
_DISK="$(df -h / 2>/dev/null | awk 'NR==2 {print $3"/"$2" ("$5")"}' || echo 'N/A')"

echo ""
echo -e "${C_BOLD}${C_CYAN}  ╔════════════════════════════════════════════════╗${C_NC}"
echo -e "${C_BOLD}${C_CYAN}  ║  VirtuaOS 1.0.1  •  Debian 13  •  x86_64      ║${C_NC}"
echo -e "${C_BOLD}${C_CYAN}  ╚════════════════════════════════════════════════╝${C_NC}"
echo ""
echo -e "  ${C_DIM}System${C_NC}"
echo -e "    ${C_BOLD}IP Address${C_NC}   : ${_IP}"
echo -e "    ${C_BOLD}Uptime${C_NC}       : ${_UPTIME}"
echo -e "    ${C_BOLD}Load${C_NC}         : ${_LOAD}"
echo -e "    ${C_BOLD}Memory${C_NC}       : ${_MEM_FREE} MB free / ${_MEM_TOTAL} MB total"
echo -e "    ${C_BOLD}Disk (/)${C_NC}     : ${_DISK}"
echo ""
echo -e "  ${C_DIM}Quick commands${C_NC}"
echo -e "    ${C_CYAN}virtuaos-status${C_NC}   — detailed system status"
echo ""
