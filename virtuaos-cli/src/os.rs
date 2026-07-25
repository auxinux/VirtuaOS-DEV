//! `virtuaos os` — VirtuaOS release status and in-place OS upgrades.
//!
//! An upgrade is driven entirely by a remote, declarative manifest fetched
//! from `dep.auxinux.ca/VIRTUAOS/<version>/upgrade.instruction` (JSON, see
//! [`UpgradeInstruction`]). This lets a VirtuaOS release ship apt package
//! changes, file replacements, one-off scripts and shell commands without
//! requiring a new `virtuaos-cli` build for every release.

use anyhow::{bail, Context, Result};
use owo_colors::OwoColorize;
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Markdown file listing the latest published VirtuaOS version.
const RELEASE_MD_URL: &str = "https://dep.auxinux.ca/VIRTUAOS/release.md";
/// Base URL under which each release's `upgrade.instruction` and assets live.
const RELEASE_BASE_URL: &str = "https://dep.auxinux.ca/VIRTUAOS";

/// Parsed key/value pairs from `/etc/os-release` (the standard
/// freedesktop.org file describing the running distribution).
#[derive(Debug)]
struct OsRelease {
    fields: HashMap<String, String>,
}

/// Result of checking the remote release manifest.
#[derive(Debug)]
struct ReleaseInfo {
    latest: String,
}

/// Declarative upgrade manifest served as `upgrade.instruction` for a given
/// VirtuaOS release. Every field is optional so a release can ship only the
/// steps it actually needs; [`apply_instruction`] executes them in a fixed
/// order (apt update -> full-upgrade -> install -> remove -> files ->
/// downloaded scripts -> commands -> reboot notice).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpgradeInstruction {
    /// Expected target version; checked against the requested version to
    /// catch a stale or mismatched manifest.
    version: Option<String>,
    reboot_required: Option<bool>,
    apt_update: Option<bool>,
    apt_full_upgrade: Option<bool>,
    apt_install: Option<Vec<String>>,
    apt_remove: Option<Vec<String>>,
    /// Files to download and install in place; `downloads` is an alias kept
    /// for older manifests and is merged with `files` at apply time.
    files: Option<Vec<FileInstruction>>,
    downloads: Option<Vec<FileInstruction>>,
    downloaded_scripts: Option<Vec<DownloadedScriptInstruction>>,
    commands: Option<Vec<CommandInstruction>>,
    /// Informational lines printed to the user before anything is applied.
    messages: Option<Vec<String>>,
}

/// One file to fetch and install onto the host filesystem.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileInstruction {
    /// Source URL; defaults to `RELEASE_BASE_URL/<version>/<dest>` when omitted.
    url: Option<String>,
    /// Destination path on the host. `dest` is an alias for `path`.
    path: Option<String>,
    dest: Option<String>,
    /// Optional `chmod` mode string (e.g. `"0755"`) applied after install.
    mode: Option<String>,
}

/// A helper script to download and execute once, then discard.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadedScriptInstruction {
    url: String,
    /// Where to save the script; defaults to the work dir with the URL's basename.
    path: Option<String>,
    args: Option<Vec<String>>,
    cwd: Option<String>,
    mode: Option<String>,
    description: Option<String>,
}

/// A raw shell command to run via `bash -lc`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommandInstruction {
    run: String,
    description: Option<String>,
}

/// Abort with a helpful message unless running as root (upgrades need to
/// write under `/etc`, `/usr`, run apt, etc.).
fn require_root() -> Result<()> {
    // geteuid() is always safe to call.
    if unsafe { libc::geteuid() } != 0 {
        bail!("run as root: sudo virtuaos os upgrade");
    }
    Ok(())
}

/// Read and parse `/etc/os-release` into a simple key/value map.
fn os_release() -> Result<OsRelease> {
    let text = fs::read_to_string("/etc/os-release")
        .context("cannot read /etc/os-release; `virtuaos os` is only available on VirtuaOS")?;
    let fields = text
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_string(), value.trim_matches('"').to_string()))
        })
        .collect();
    Ok(OsRelease { fields })
}

/// Confirm the host is actually VirtuaOS before touching anything.
/// Checks the `/etc/os-release` `ID`/`NAME` fields as well as the
/// `/etc/virtuaos-release` marker file, since either can be present
/// depending on how the release was branded.
fn ensure_virtuaos() -> Result<OsRelease> {
    let rel = os_release()?;
    let id = rel.field("ID").to_lowercase();
    let name = format!("{} {}", rel.field("NAME"), rel.field("PRETTY_NAME")).to_lowercase();
    if id == "virtuaos" || name.contains("virtuaos") || Path::new("/etc/virtuaos-release").exists()
    {
        return Ok(rel);
    }
    bail!("`virtuaos os` is not available on this distribution; this host is not VirtuaOS");
}

impl OsRelease {
    fn field(&self, key: &str) -> String {
        self.fields.get(key).cloned().unwrap_or_default()
    }

    /// Best-effort detection of the installed VirtuaOS version. Prefers the
    /// dedicated `/etc/virtuaos-version` stamp file (kept in sync by
    /// `virtuaos apply-branding`, see `branding.rs`) and falls back to
    /// scanning standard `/etc/os-release` fields, then finally `"0.0.0"`
    /// if nothing looks like a version number.
    fn installed_version(&self) -> String {
        if let Ok(version) = fs::read_to_string("/etc/virtuaos-version") {
            if let Some(version) = extract_version(version.trim()) {
                return version;
            }
        }
        for key in ["VERSION_ID", "VERSION", "PRETTY_NAME"] {
            if let Some(version) = extract_version(&self.field(key)) {
                return version;
            }
        }
        "0.0.0".to_string()
    }
}

/// `virtuaos os status` — print the installed vs. latest published version
/// and hint at an upgrade when one is available.
pub fn status() -> Result<()> {
    let rel = ensure_virtuaos()?;
    let installed = rel.installed_version();
    println!("{}", "── VirtuaOS release ──────────────────────".dimmed());
    println!("  {:<12} {}", "installed", installed.bold());

    let remote = fetch_release_info()?;
    println!("  {:<12} {}", "available", remote.latest.bold());

    match compare_versions(&installed, &remote.latest) {
        Some(Ordering::Less) => {
            println!(
                "\n{} New VirtuaOS release available: {}",
                "⬆".yellow(),
                remote.latest.bold()
            );
            println!("  Upgrade with: {}", "vos os upgrade".bold());
        }
        Some(Ordering::Equal) => println!("\n{} VirtuaOS is up to date", "✓".green()),
        Some(Ordering::Greater) => println!(
            "\n{} installed version is newer than the public release manifest",
            "ℹ".blue()
        ),
        None => println!(
            "\n{} could not compare versions; check {}",
            "?".yellow(),
            RELEASE_MD_URL
        ),
    }
    Ok(())
}

/// `virtuaos os upgrade` — fetch the target release's `upgrade.instruction`
/// manifest and apply it. `target_version` pins a specific release instead
/// of "latest"; `reapply` forces re-running the manifest even when the host
/// is already on the target version (useful to refresh branding/files after
/// a manual fix); `yes` skips the confirmation prompt.
pub fn upgrade(target_version: Option<String>, reapply: bool, yes: bool) -> Result<()> {
    require_root()?;
    let rel = ensure_virtuaos()?;
    let installed = rel.installed_version();
    let target = match target_version {
        Some(v) => v,
        None => fetch_release_info()?.latest,
    };

    println!("{}", "── VirtuaOS upgrade ──────────────────────".dimmed());
    println!("  {:<12} {}", "installed", installed.bold());
    println!("  {:<12} {}", "target", target.bold());

    if compare_versions(&installed, &target) == Some(Ordering::Equal) && !reapply {
        println!("\n{} VirtuaOS is already at {}", "✓".green(), target);
        println!(
            "{} Use {} to reapply the same release and refresh branding/files.",
            "ℹ".blue(),
            "vos os upgrade --reapply".bold()
        );
        return Ok(());
    }

    if compare_versions(&installed, &target) == Some(Ordering::Equal) && reapply {
        println!(
            "\n{} Reapplying VirtuaOS {} to refresh files and branding…",
            "ℹ".blue(),
            target.bold()
        );
    }

    if !yes && !confirm(&format!("\nUpgrade VirtuaOS to {target}?"))? {
        println!("Aborted.");
        return Ok(());
    }

    let workdir = prepare_workdir(&target)?;
    let instruction_url = format!("{RELEASE_BASE_URL}/{target}/upgrade.instruction");
    let instruction_path = workdir.join("upgrade.instruction");
    download(&instruction_url, &instruction_path)?;

    let instruction_text = fs::read_to_string(&instruction_path)
        .with_context(|| format!("cannot read {}", instruction_path.display()))?;
    let instruction: UpgradeInstruction = serde_json::from_str(&instruction_text)
        .with_context(|| format!("invalid JSON in {instruction_url}"))?;

    // Guard against a manifest served for the wrong version (e.g. a stale
    // CDN cache or a copy/paste mistake when publishing the release).
    if let Some(version) = &instruction.version {
        if version != &target {
            bail!("upgrade.instruction version mismatch: expected {target}, got {version}");
        }
    }

    apply_instruction(&target, &workdir, instruction)?;
    println!("\n{} VirtuaOS upgrade instructions applied.", "✓".green());
    println!("Run {} to verify.", "vos os status".bold());
    Ok(())
}

/// Fetch and parse `release.md` to find the latest published version.
fn fetch_release_info() -> Result<ReleaseInfo> {
    let text = fetch_text(RELEASE_MD_URL)?;
    let latest = parse_latest_version(&text)
        .with_context(|| format!("cannot find a release version in {RELEASE_MD_URL}"))?;
    Ok(ReleaseInfo { latest })
}

/// `release.md` is free-form prose, not structured data. Prefer a version
/// found on a line that reads like "latest: 1.2.3" / "current version ...",
/// and only fall back to the first digit-dot sequence found anywhere in the
/// file if no such labelled line exists.
fn parse_latest_version(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        if lower.starts_with("latest")
            || lower.starts_with("version")
            || lower.starts_with("current")
            || lower.starts_with("stable")
        {
            if let Some(version) = extract_version(trimmed) {
                return Some(version);
            }
        }
    }
    text.lines().find_map(extract_version)
}

/// Scan `text` for the first substring that looks like a dotted version
/// number (at least one digit, one `.`, e.g. `1.0`, `1.2.3`) and return it.
/// Used to pull a version out of otherwise free-form strings such as
/// `PRETTY_NAME="VirtuaOS 1.2.3"` or a `release.md` sentence.
fn extract_version(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    for start in 0..bytes.len() {
        if !bytes[start].is_ascii_digit() {
            continue;
        }
        let mut end = start;
        let mut dots = 0;
        while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
            if bytes[end] == b'.' {
                dots += 1;
            }
            end += 1;
        }
        if dots >= 1 {
            return Some(text[start..end].trim_end_matches('.').to_string());
        }
    }
    None
}

/// Compare two version strings component-by-component as integers (so `2.9`
/// < `2.10`, unlike a plain string comparison). Missing trailing components
/// are treated as `0` (`1.2` == `1.2.0`). Returns `None` if either string
/// doesn't contain a parseable version.
fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let av = parse_version_parts(a)?;
    let bv = parse_version_parts(b)?;
    let len = av.len().max(bv.len());
    for i in 0..len {
        let left = *av.get(i).unwrap_or(&0);
        let right = *bv.get(i).unwrap_or(&0);
        match left.cmp(&right) {
            Ordering::Equal => {}
            ord => return Some(ord),
        }
    }
    Some(Ordering::Equal)
}

/// Split a version string into its numeric dot-separated components.
fn parse_version_parts(version: &str) -> Option<Vec<u64>> {
    let version = extract_version(version).unwrap_or_else(|| version.to_string());
    let parts: Option<Vec<u64>> = version
        .split('.')
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u64>().ok())
        .collect();
    parts.filter(|v| !v.is_empty())
}

/// Create a fresh, empty scratch directory for downloading a given release's
/// manifest and assets. Namespaced by version and PID so concurrent/retried
/// upgrades don't collide, and wiped first in case a previous run left it dirty.
fn prepare_workdir(version: &str) -> Result<PathBuf> {
    let dir = PathBuf::from(format!(
        "/tmp/virtuaos-upgrade-{version}-{}",
        std::process::id()
    ));
    if dir.exists() {
        fs::remove_dir_all(&dir).with_context(|| format!("cannot clean {}", dir.display()))?;
    }
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    Ok(dir)
}

/// Execute an [`UpgradeInstruction`] step by step, in a fixed order:
/// apt update -> full-upgrade -> install -> remove -> files/downloads ->
/// downloaded scripts -> commands -> reboot notice. Each step is a no-op if
/// its field is absent, so a manifest only pays for what it declares.
fn apply_instruction(version: &str, workdir: &Path, instruction: UpgradeInstruction) -> Result<()> {
    if let Some(messages) = instruction.messages {
        for message in messages {
            println!("{} {}", "ℹ".blue(), message);
        }
    }

    if instruction.apt_update.unwrap_or(false) {
        println!("\n{}", "→ Refreshing apt indexes…".bold());
        run_inherit("apt-get", &["update".to_string()])?;
    }

    if instruction.apt_full_upgrade.unwrap_or(false) {
        println!("\n{}", "→ Applying apt full-upgrade…".bold());
        run_inherit("apt-get", &["full-upgrade".to_string(), "-y".to_string()])?;
    }

    if let Some(pkgs) = instruction.apt_install {
        if !pkgs.is_empty() {
            println!("\n{}", "→ Installing required packages…".bold());
            let mut args = vec!["install".to_string(), "-y".to_string()];
            args.extend(pkgs);
            run_inherit("apt-get", &args)?;
        }
    }

    if let Some(pkgs) = instruction.apt_remove {
        if !pkgs.is_empty() {
            println!("\n{}", "→ Removing obsolete packages…".bold());
            let mut args = vec!["remove".to_string(), "-y".to_string()];
            args.extend(pkgs);
            run_inherit("apt-get", &args)?;
        }
    }

    for file in instruction
        .files
        .unwrap_or_default()
        .into_iter()
        .chain(instruction.downloads.unwrap_or_default())
    {
        apply_file(version, workdir, file)?;
    }

    if let Some(scripts) = instruction.downloaded_scripts {
        for script in scripts {
            apply_downloaded_script(workdir, script)?;
        }
    }

    if let Some(commands) = instruction.commands {
        for cmd in commands {
            if let Some(description) = &cmd.description {
                println!("\n{} {}", "→".cyan(), description.bold());
            } else {
                println!("\n{} {}", "→".cyan(), cmd.run.bold());
            }
            run_shell(&cmd.run)?;
        }
    }

    if instruction.reboot_required.unwrap_or(false) {
        println!(
            "\n{} Reboot required to finish this VirtuaOS upgrade: {}",
            "⚠".yellow(),
            "sudo reboot".bold()
        );
    }
    Ok(())
}

/// Download a helper script into the work dir (or a caller-specified path),
/// make it executable and run it once with the given args/cwd. Used for
/// upgrade steps too involved to express as a single shell `command`.
fn apply_downloaded_script(workdir: &Path, script: DownloadedScriptInstruction) -> Result<()> {
    let local_path = match script.path.as_deref() {
        Some(path) => PathBuf::from(path),
        None => {
            let name = Path::new(&script.url)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("upgrade-script.sh");
            workdir.join(name)
        }
    };

    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    if let Some(description) = &script.description {
        println!("\n{} {}", "→".cyan(), description.bold());
    }

    download(&script.url, &local_path)?;

    let mode = script.mode.unwrap_or_else(|| "0755".to_string());
    run_inherit("chmod", &[mode, local_path.display().to_string()])?;

    let cwd = script.cwd.unwrap_or_else(|| workdir.display().to_string());
    let args = script.args.unwrap_or_default();

    println!(
        "{} {}{}",
        "→".cyan(),
        local_path.display(),
        if args.is_empty() {
            String::new()
        } else {
            format!(" {}", args.join(" "))
        }
    );

    let status = Command::new(&local_path)
        .current_dir(&cwd)
        .args(&args)
        .status()
        .with_context(|| {
            format!(
                "failed to launch downloaded script {}",
                local_path.display()
            )
        })?;
    if !status.success() {
        bail!(
            "downloaded script exited with {status}: {}",
            local_path.display()
        );
    }
    Ok(())
}

/// Download one [`FileInstruction`] to a temp path in the work dir, then
/// copy it into place at `dest` (creating parent directories as needed) and
/// apply an optional `chmod` mode. Downloading to a temp path first avoids
/// leaving a half-written file at `dest` if the transfer fails midway.
fn apply_file(version: &str, workdir: &Path, file: FileInstruction) -> Result<()> {
    let dest = file
        .path
        .or(file.dest)
        .context("file instruction requires `path` or `dest`")?;
    let url = file.url.unwrap_or_else(|| {
        format!(
            "{RELEASE_BASE_URL}/{version}/{}",
            dest.trim_start_matches('/')
        )
    });
    let filename = Path::new(&dest)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    let tmp = workdir.join(filename);

    println!("{} {} -> {}", "→".cyan(), url, dest);
    download(&url, &tmp)?;

    if let Some(parent) = Path::new(&dest).parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::copy(&tmp, &dest).with_context(|| format!("cannot install {dest}"))?;

    if let Some(mode) = file.mode {
        run_inherit("chmod", &[mode, dest.clone()])?;
    }
    Ok(())
}

/// GET `url` via `curl` and return the response body as a string.
fn fetch_text(url: &str) -> Result<String> {
    let out = Command::new("curl")
        .args(["-fsSL", url])
        .output()
        .with_context(|| format!("failed to launch curl for {url}"))?;
    if !out.status.success() {
        bail!("failed to fetch {url}");
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Download `url` to `dest` via `curl`, failing the whole upgrade on error.
fn download(url: &str, dest: &Path) -> Result<()> {
    let status = Command::new("curl")
        .arg("-fsSL")
        .arg(url)
        .arg("-o")
        .arg(dest)
        .status()
        .with_context(|| format!("failed to launch curl for {url}"))?;
    if !status.success() {
        bail!("failed to download {url}");
    }
    Ok(())
}

/// Run `program` with its stdout/stderr inherited (so the user sees live
/// output, e.g. apt's progress bar), failing on a non-zero exit status.
fn run_inherit(program: &str, args: &[String]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("failed to launch `{program}`"))?;
    if !status.success() {
        bail!("`{program}` exited with {status}");
    }
    Ok(())
}

/// Run an arbitrary shell command string (from a manifest's `commands`
/// list) via `bash -lc`, so it can use pipes, globs, etc.
fn run_shell(command: &str) -> Result<()> {
    let status = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .status()
        .with_context(|| format!("failed to launch shell command `{command}`"))?;
    if !status.success() {
        bail!("shell command exited with {status}: {command}");
    }
    Ok(())
}

/// Prompt the user for a yes/no answer, defaulting to "no" on empty input.
/// Accepts English and French affirmatives (`y`/`yes`, `o`/`oui`).
fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt} [y/N] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_lowercase().as_str(),
        "y" | "yes" | "o" | "oui"
    ))
}
