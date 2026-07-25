//! Data model: the answer set, the menu option lists (mirrored from the bash
//! installer so behaviour is identical), and internal-disk detection.

use std::process::Command;

/// A selectable option: stable `value` written to the conf, human `label` shown.
#[derive(Clone)]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
}

/// Wizard UI language (also seeds the locale default).
pub const UI_LANGS: &[Choice] = &[
    Choice { value: "en", label: "English" },
    Choice { value: "fr", label: "Français" },
];

/// System locale — mirrors choose_language() in virtuaos-install.
pub const LOCALES: &[Choice] = &[
    Choice { value: "en_US.UTF-8", label: "English (US)" },
    Choice { value: "en_GB.UTF-8", label: "English (UK)" },
    Choice { value: "fr_FR.UTF-8", label: "Français (France)" },
    Choice { value: "fr_CA.UTF-8", label: "Français (Canada)" },
    Choice { value: "es_ES.UTF-8", label: "Español (España)" },
    Choice { value: "de_DE.UTF-8", label: "Deutsch (Deutschland)" },
    Choice { value: "it_IT.UTF-8", label: "Italiano (Italia)" },
    Choice { value: "pt_BR.UTF-8", label: "Português (Brasil)" },
    Choice { value: "pt_PT.UTF-8", label: "Português (Portugal)" },
    Choice { value: "nl_NL.UTF-8", label: "Nederlands" },
];

/// Keyboard layout — mirrors choose_keyboard().
pub const KEYMAPS: &[Choice] = &[
    Choice { value: "us", label: "US (QWERTY)" },
    Choice { value: "fr", label: "Français (AZERTY)" },
    Choice { value: "ca", label: "Canadien Français" },
    Choice { value: "ca-multix", label: "Canadien Multilingue" },
    Choice { value: "es", label: "Español" },
    Choice { value: "de", label: "Deutsch (QWERTZ)" },
    Choice { value: "it", label: "Italiano" },
    Choice { value: "pt-br", label: "Português (Brasil)" },
    Choice { value: "pt", label: "Português (Portugal)" },
    Choice { value: "be", label: "Belge" },
    Choice { value: "ch", label: "Suisse / Swiss" },
];

/// Timezone — mirrors choose_timezone().
pub const TIMEZONES: &[Choice] = &[
    Choice { value: "UTC", label: "UTC" },
    Choice { value: "America/Toronto", label: "America/Toronto (Eastern, Canada)" },
    Choice { value: "America/Montreal", label: "America/Montreal (Eastern, Canada)" },
    Choice { value: "America/New_York", label: "America/New_York (Eastern, USA)" },
    Choice { value: "America/Chicago", label: "America/Chicago (Central, USA)" },
    Choice { value: "America/Denver", label: "America/Denver (Mountain, USA)" },
    Choice { value: "America/Los_Angeles", label: "America/Los_Angeles (Pacific, USA)" },
    Choice { value: "Europe/Paris", label: "Europe/Paris" },
    Choice { value: "Europe/London", label: "Europe/London" },
    Choice { value: "Europe/Berlin", label: "Europe/Berlin" },
    Choice { value: "Europe/Madrid", label: "Europe/Madrid" },
    Choice { value: "Europe/Rome", label: "Europe/Rome" },
    Choice { value: "Asia/Tokyo", label: "Asia/Tokyo" },
    Choice { value: "Australia/Sydney", label: "Australia/Sydney" },
];

/// An internal disk eligible for install.
#[derive(Clone)]
pub struct Disk {
    pub dev: String,
    pub size: String,
    pub model: String,
}

/// Detect internal disks via the same helper the bash installer uses.
/// Output format per line: `dev<TAB>size<TAB>model<TAB>internal`.
pub fn detect_disks() -> Vec<Disk> {
    let out = Command::new("/usr/local/sbin/virtuaos-detect-disks").output();
    let raw = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => return Vec::new(),
    };
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut f = l.split('\t');
            Disk {
                dev: f.next().unwrap_or("").trim().to_string(),
                size: f.next().unwrap_or("").trim().to_string(),
                model: f.next().unwrap_or("").trim().to_string(),
            }
        })
        .filter(|d| !d.dev.is_empty())
        .collect()
}

/// Password policy shared with the bash installer and the web panel:
/// >= 12 chars, at least one letter and one digit.
pub fn password_ok(pw: &str) -> bool {
    pw.chars().count() >= 12
        && pw.chars().any(|c| c.is_ascii_alphabetic())
        && pw.chars().any(|c| c.is_ascii_digit())
}

/// Hostname policy: `^[a-z0-9][a-z0-9-]{0,62}$` (lowercased).
pub fn hostname_ok(h: &str) -> bool {
    let h = h.trim();
    if h.is_empty() || h.len() > 63 {
        return false;
    }
    let bytes = h.as_bytes();
    let first_ok = bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit();
    first_ok
        && h.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// All wizard answers, ready to serialise into the backend conf file.
#[derive(Default, Clone)]
pub struct Answers {
    pub ui_lang: String,
    pub lang_code: String,
    pub keymap: String,
    pub hostname: String,
    pub tz: String,
    pub root_password: String,
    pub admin_password: String,
    pub disk: String,
    // ── Network ──────────────────────────────────────────────────────────────
    /// Interface device to configure (e.g. enp3s0, wlan0). Empty = skip (DHCP
    /// on whatever the live env already brought up).
    pub net_iface: String,
    /// "ethernet" | "wifi" (matches the NM connection type).
    pub net_kind: String,
    pub wifi_ssid: String,
    pub wifi_password: String,
    /// "dhcp" | "static".
    pub net_method: String,
    /// Static only: "192.168.1.50/24".
    pub net_address: String,
    pub net_gateway: String,
    /// Static only: space- or comma-separated DNS servers.
    pub net_dns: String,
}

impl Answers {
    /// Serialise to the `key=value` form load_answers() parses. The value is
    /// everything after the first '=', so passwords/SSIDs may safely contain '='.
    pub fn to_conf(&self) -> String {
        format!(
            "ui_lang={}\nlang_code={}\nkeymap={}\nhostname={}\ntz={}\ndisk={}\n\
             net_iface={}\nnet_kind={}\nwifi_ssid={}\nwifi_password={}\n\
             net_method={}\nnet_address={}\nnet_gateway={}\nnet_dns={}\n\
             root_password={}\nadmin_password={}\n",
            self.ui_lang,
            self.lang_code,
            self.keymap,
            self.hostname,
            self.tz,
            self.disk,
            self.net_iface,
            self.net_kind,
            self.wifi_ssid,
            self.wifi_password,
            self.net_method,
            self.net_address,
            self.net_gateway,
            self.net_dns,
            self.root_password,
            self.admin_password,
        )
    }
}

// ── Network model ───────────────────────────────────────────────────────────

/// DHCP vs static address assignment.
pub const NET_METHODS: &[Choice] = &[
    Choice { value: "dhcp", label: "Automatique (DHCP)" },
    Choice { value: "static", label: "IP fixe (manuelle)" },
];

#[derive(Clone, Copy, PartialEq)]
pub enum IfKind {
    Ethernet,
    Wifi,
}

/// A network interface eligible for configuration.
#[derive(Clone)]
pub struct NetIface {
    pub dev: String,
    pub kind: IfKind,
    pub state: String,
}

impl NetIface {
    pub fn is_wifi(&self) -> bool {
        self.kind == IfKind::Wifi
    }
    pub fn kind_str(&self) -> &'static str {
        match self.kind {
            IfKind::Wifi => "wifi",
            IfKind::Ethernet => "ethernet",
        }
    }
}

/// A scanned WiFi network.
#[derive(Clone)]
pub struct WifiNet {
    pub ssid: String,
    pub signal: u8,
    pub secured: bool,
}

/// `nmcli -t` terse output escapes the ':' separator as '\:'. Split a line into
/// fields honoring that escaping.
fn nm_split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut esc = false;
    for c in line.chars() {
        if esc {
            cur.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else if c == ':' {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

/// Detect configurable network interfaces. Prefers NetworkManager (so the
/// classification matches what we'll configure), falls back to sysfs so the
/// list is never empty just because nmcli hiccupped.
pub fn detect_ifaces() -> Vec<NetIface> {
    // nmcli -t -f DEVICE,TYPE,STATE device
    if let Ok(o) = Command::new("nmcli")
        .args(["-t", "-f", "DEVICE,TYPE,STATE", "device"])
        .output()
    {
        if o.status.success() {
            let raw = String::from_utf8_lossy(&o.stdout);
            let mut v: Vec<NetIface> = raw
                .lines()
                .filter_map(|l| {
                    let f = nm_split(l);
                    if f.len() < 3 {
                        return None;
                    }
                    let (dev, ty, st) = (f[0].trim(), f[1].trim(), f[2].trim());
                    let kind = match ty {
                        "wifi" => IfKind::Wifi,
                        "ethernet" => IfKind::Ethernet,
                        _ => return None, // skip loopback, bridge, tun, etc.
                    };
                    if dev.is_empty() || dev == "lo" {
                        return None;
                    }
                    Some(NetIface {
                        dev: dev.to_string(),
                        kind,
                        state: st.to_string(),
                    })
                })
                .collect();
            if !v.is_empty() {
                v.sort_by(|a, b| a.dev.cmp(&b.dev));
                return v;
            }
        }
    }
    detect_ifaces_sysfs()
}

/// Fallback interface detection straight from `/sys/class/net`, used when
/// `nmcli` is unavailable or returns nothing.
fn detect_ifaces_sysfs() -> Vec<NetIface> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/sys/class/net") {
        for e in rd.flatten() {
            let dev = e.file_name().to_string_lossy().to_string();
            if dev == "lo" || dev.starts_with("vir") || dev.starts_with("br") || dev.starts_with("docker") {
                continue;
            }
            let is_wifi = std::path::Path::new(&format!("/sys/class/net/{dev}/wireless")).exists()
                || std::path::Path::new(&format!("/sys/class/net/{dev}/phy80211")).exists();
            // Only physical NICs (skip tun/tap/veth which lack a device link to a bus).
            let phys = std::path::Path::new(&format!("/sys/class/net/{dev}/device")).exists();
            if !phys && !is_wifi {
                continue;
            }
            v.push(NetIface {
                dev,
                kind: if is_wifi { IfKind::Wifi } else { IfKind::Ethernet },
                state: String::new(),
            });
        }
    }
    v.sort_by(|a, b| a.dev.cmp(&b.dev));
    v
}

/// Scan WiFi networks on a device via NetworkManager (triggers a fresh rescan).
pub fn scan_wifi(dev: &str) -> Vec<WifiNet> {
    let _ = Command::new("nmcli")
        .args(["device", "wifi", "rescan", "ifname", dev])
        .output();
    let out = Command::new("nmcli")
        .args([
            "-t",
            "-f",
            "SSID,SIGNAL,SECURITY",
            "device",
            "wifi",
            "list",
            "ifname",
            dev,
        ])
        .output();
    let raw = match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).into_owned(),
        _ => return Vec::new(),
    };
    let mut seen = std::collections::HashSet::new();
    let mut nets: Vec<WifiNet> = Vec::new();
    for l in raw.lines() {
        let f = nm_split(l);
        if f.is_empty() {
            continue;
        }
        let ssid = f.first().map(|s| s.trim()).unwrap_or("");
        if ssid.is_empty() || !seen.insert(ssid.to_string()) {
            continue; // skip hidden/empty + dedupe by SSID (strongest first)
        }
        let signal = f.get(1).and_then(|s| s.trim().parse::<u8>().ok()).unwrap_or(0);
        let sec = f.get(2).map(|s| s.trim()).unwrap_or("");
        let secured = !(sec.is_empty() || sec == "--");
        nets.push(WifiNet {
            ssid: ssid.to_string(),
            signal,
            secured,
        });
    }
    nets.sort_by(|a, b| b.signal.cmp(&a.signal));
    nets
}

/// Validate "A.B.C.D/PREFIX".
pub fn cidr_ok(s: &str) -> bool {
    let s = s.trim();
    let Some((ip, pfx)) = s.split_once('/') else {
        return false;
    };
    ip_ok(ip) && pfx.parse::<u8>().map(|p| p <= 32).unwrap_or(false)
}

/// Validate a dotted-quad IPv4 address.
pub fn ip_ok(s: &str) -> bool {
    let s = s.trim();
    let parts: Vec<&str> = s.split('.').collect();
    parts.len() == 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.parse::<u8>().is_ok())
}

/// Validate a (possibly empty) list of DNS servers separated by space/comma.
pub fn dns_list_ok(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return true;
    }
    s.split(|c| c == ',' || c == ' ')
        .filter(|p| !p.trim().is_empty())
        .all(|p| ip_ok(p))
}
