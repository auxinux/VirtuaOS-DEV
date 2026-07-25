//! `virtuaos` — VirtuaOS host & update manager.
//!
//! Phase 1 scope: the UPDATER. It orchestrates apt to update the three AuxiNux
//! packages served from dep.auxinux.ca/VIRTUA (kernel-virtua, auxinux-virtua,
//! virtuaos-cli) plus the base Debian system, and restarts the Virtua services
//! when the panel is upgraded. Host-management TUI (network/storage/services)
//! comes in a later phase.

mod branding;
mod network;
mod os;
mod setup;
mod system;
mod update;
mod vdm;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "virtuaos",
    version,
    about = "VirtuaOS — host & update manager",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Show component versions and service health (this is the default).
    Status,
    /// Print VirtuaOS / kernel / Virtua / CLI versions only.
    Version,
    /// Update VirtuaOS, the AuxiNux kernel and the Virtua panel via apt.
    Update {
        /// Only show what would be upgraded; apply nothing.
        #[arg(long)]
        check: bool,
        /// Apply without the confirmation prompt.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Watch the live progress of an in-flight Virtua install/upgrade.
    Setup {
        /// Print a single snapshot and exit (for scripts / non-TTY).
        #[arg(long)]
        once: bool,
    },
    /// Show or configure host networking (Ethernet / WiFi, DHCP / static).
    Network {
        #[command(subcommand)]
        cmd: Option<NetworkCmd>,
    },
    /// Manage VirtuaOS image/release upgrades.
    Os {
        #[command(subcommand)]
        cmd: Option<OsCmd>,
    },
    /// Install and manage the single VDM instance for this Virtua cluster.
    Vdm {
        #[command(subcommand)]
        cmd: Option<VdmCmd>,
    },
    /// (internal) Stamp this cli's VirtuaOS release onto /etc/os-release + issue.
    /// Run by the virtuaos-cli package postinst so `virtuaos update` promotes the
    /// host's version when the cli is upgraded.
    #[command(name = "apply-branding", hide = true)]
    ApplyBranding,
}

#[derive(Subcommand)]
enum NetworkCmd {
    /// Show interfaces, link state and IP addresses (default).
    Status,
    /// Interactive wizard: pick a NIC, join WiFi, choose DHCP or a static IP.
    Config,
}

#[derive(Subcommand)]
enum OsCmd {
    /// Show installed VirtuaOS version and check the latest release manifest.
    Status,
    /// Upgrade VirtuaOS from dep.auxinux.ca/VIRTUAOS/<version>/.
    Upgrade {
        /// Upgrade to a specific VirtuaOS version instead of the latest one.
        #[arg(long = "to")]
        target: Option<String>,
        /// Reapply the target release even if the installed version already matches.
        #[arg(long)]
        reapply: bool,
        /// Apply without the confirmation prompt.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Alias for upgrade.
    Update {
        /// Upgrade to a specific VirtuaOS version instead of the latest one.
        #[arg(long = "to")]
        target: Option<String>,
        /// Reapply the target release even if the installed version already matches.
        #[arg(long)]
        reapply: bool,
        /// Apply without the confirmation prompt.
        #[arg(short = 'y', long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
enum VdmCmd {
    /// Install VDM in its managed LXC. Refuses when the cluster already has VDM.
    Install {
        /// Enable the HA policy after installation (requires HA prerequisites).
        #[arg(long)]
        ha: bool,
        /// LXC network bridge.
        #[arg(long, default_value = "vmbr0")]
        bridge: String,
        /// DHCP or a static address in CIDR form.
        #[arg(long, default_value = "dhcp")]
        ipv4: String,
        /// Gateway used with a static address.
        #[arg(long)]
        gateway: Option<String>,
    },
    /// Update the existing VDM while preserving its database and configuration.
    Update,
    /// Remove the managed VDM LXC from this cluster.
    Uninstall {
        /// Do not ask for confirmation.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Move the only VDM instance to another Virtua node over SSH.
    #[command(name = "movenode")]
    MoveNode {
        /// Target node hostname or SSH address.
        target: String,
        /// Do not ask for confirmation.
        #[arg(short = 'y', long)]
        yes: bool,
    },
    /// Show VDM placement, runtime health and HA state.
    Status,
    /// Enable, disable or inspect automatic HA policy.
    Ha {
        #[command(subcommand)]
        cmd: VdmHaCmd,
    },
}

#[derive(Subcommand)]
enum VdmHaCmd {
    /// Enable HA after validating the cluster prerequisites.
    Enable,
    /// Disable automatic failover without uninstalling VDM.
    Disable,
    /// Show the HA configuration.
    Status,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Status) {
        Command::Status => system::print_status(),
        Command::Version => system::print_versions(),
        Command::Update { check, yes } => update::run(check, yes),
        Command::Setup { once } => setup::run(once),
        Command::Network { cmd } => network::run(matches!(cmd, Some(NetworkCmd::Config))),
        Command::Os { cmd } => match cmd.unwrap_or(OsCmd::Status) {
            OsCmd::Status => os::status(),
            OsCmd::Upgrade {
                target,
                reapply,
                yes,
            }
            | OsCmd::Update {
                target,
                reapply,
                yes,
            } => os::upgrade(target, reapply, yes),
        },
        Command::Vdm { cmd } => match cmd.unwrap_or(VdmCmd::Status) {
            VdmCmd::Install { ha, bridge, ipv4, gateway } => {
                vdm::install(ha, &bridge, &ipv4, gateway.as_deref())
            }
            VdmCmd::Update => vdm::update(),
            VdmCmd::Uninstall { yes } => vdm::uninstall(yes),
            VdmCmd::MoveNode { target, yes } => vdm::move_node(&target, yes),
            VdmCmd::Status => vdm::status(),
            VdmCmd::Ha { cmd } => match cmd {
                VdmHaCmd::Enable => vdm::ha("enable"),
                VdmHaCmd::Disable => vdm::ha("disable"),
                VdmHaCmd::Status => vdm::ha("status"),
            },
        },
        Command::ApplyBranding => branding::apply(),
    }
}
