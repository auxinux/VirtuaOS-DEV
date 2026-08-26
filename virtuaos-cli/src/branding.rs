//! VirtuaOS release identity.
//!
//! The VirtuaOS "version" shown in /etc/os-release, /etc/issue and /etc/issue.net
//! is NOT owned by any single apt package, so `apt upgrade` alone can never bump
//! it on an installed host. This module makes `virtuaos-cli` the release-identity
//! carrier: the constant below is stamped into the branding files by
//! `virtuaos apply-branding`, which the package's postinst runs on every upgrade.
//! Result: an existing 1.0.x host that runs `virtuaos update` upgrades the cli
//! package, whose postinst promotes the host to this release.
//!
//! KEEP IN SYNC (bump together every release) with:
//!   • VirtuaOS/VERSION
//!   • VirtuaOS/build.sh                          (VIRTUAOS_VERSION)
//!   • VirtuaOS/livecd/config.sh                  (--iso-volume / --image-name)
//!   • VirtuaOS/livecd/hooks/normal/0100-branding.hook.chroot (VIRTUAOS_VERSION + /etc/debian_version)
//!   • VirtuaOS/livecd/bootloaders/grub-{pc,efi}/grub.cfg (menu labels + volume label)
//!   • VirtuaOS/livecd/includes.chroot/usr/local/sbin/virtuaos-install (VERSION)
//!
//! The login banner (/etc/profile.d/virtuaos.sh) and `virtuaos-status` are
//! DYNAMIC: they read /etc/virtuaos-version at runtime, so they follow upgrades
//! automatically and must NOT be bumped by hand.

use anyhow::{bail, Context, Result};
use std::fs;

/// Full VirtuaOS release string.
pub const VIRTUAOS_RELEASE: &str = "1.0.3-BETA";

const ISSUE_BANNER: &str = r#"
  ██╗   ██╗██╗██████╗ ████████╗██╗   ██╗ █████╗  ██████╗ ███████╗
  ██║   ██║██║██╔══██╗╚══██╔══╝██║   ██║██╔══██╗██╔═══██╗██╔════╝
  ██║   ██║██║██████╔╝   ██║   ██║   ██║███████║██║   ██║███████╗
  ╚██╗ ██╔╝██║██╔══██╗   ██║   ██║   ██║██╔══██║██║   ██║╚════██║
   ╚████╔╝ ██║██║  ██║   ██║   ╚██████╔╝██║  ██║╚██████╔╝███████║
    ╚═══╝  ╚═╝╚═╝  ╚═╝   ╚═╝    ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝
"#;

/// Numeric part only, e.g. "1.0.2" from "1.0.3-BETA".
pub fn short_version() -> &'static str {
    VIRTUAOS_RELEASE.split('-').next().unwrap_or(VIRTUAOS_RELEASE)
}

/// (Re)write the release-identity files to VIRTUAOS_RELEASE. Idempotent; root only.
pub fn apply() -> Result<()> {
    if unsafe { libc::geteuid() } != 0 {
        bail!("this must run as root (apply-branding)");
    }
    let v = VIRTUAOS_RELEASE;

    let os_release = format!(
        "PRETTY_NAME=\"VirtuaOS {v} (Debian 13 Trixie)\"\n\
         NAME=\"VirtuaOS\"\n\
         VERSION=\"{v}\"\n\
         VERSION_ID=\"{v}\"\n\
         VERSION_CODENAME=\"trixie\"\n\
         ID=\"virtuaos\"\n\
         ID_LIKE=\"debian\"\n\
         HOME_URL=\"https://auxinux.io\"\n\
         SUPPORT_URL=\"https://auxinux.io/support\"\n\
         BUG_REPORT_URL=\"https://github.com/auxinux/virtuaos/issues\"\n\
         DEBIAN_CODENAME=\"trixie\"\n"
    );
    fs::write("/etc/os-release", os_release).context("writing /etc/os-release")?;
    fs::write("/etc/virtuaos-version", format!("{}\n", short_version()))
        .context("writing /etc/virtuaos-version")?;

    fs::write(
        "/etc/issue",
        format!("{ISSUE_BANNER}\n  Version {v}  •  Powered by Debian 13 Trixie  •  x86_64\n\n"),
    )
    .context("writing /etc/issue")?;
    fs::write(
        "/etc/issue.net",
        format!("VirtuaOS {v} — Powered by Debian 13 Trixie\nWeb Panel: https://<IP>:8441\n"),
    )
    .context("writing /etc/issue.net")?;
    Ok(())
}
