# virtuaos-cli

The `virtuaos` command — VirtuaOS host & update manager (Rust).

## Scope (phased)

- **Phase 1 (this):** updater. `virtuaos update` orchestrates apt to upgrade the
  AuxiNux packages from `dep.auxinux.ca/VIRTUA` (`kernel-virtua`, `auxinux-virtua`,
  `virtuaos-cli`) plus the base Debian system, and restarts the Virtua services
  when the panel is upgraded.
- **Phase 4 (later):** host-management TUI (network / storage / services / logs)
  with `ratatui`.

This binary is **separate** from the node CLI `virtua` (which drives VM/LXC/Docker
consoles in the panel). Different scopes on purpose.

## Commands

```
virtuaos                 # = virtuaos status
virtuaos status          # versions + service health + available updates
virtuaos version         # versions only
virtuaos update          # check + confirm + apply (apt) + restart services
virtuaos update --check  # show what would change, apply nothing
virtuaos update -y       # apply without prompting
virtuaos os status       # VirtuaOS release version + latest /VIRTUAOS release
virtuaos os upgrade      # apply /VIRTUAOS/<version>/upgrade.instruction
virtuaos os upgrade --to 0.1.0 -y
virtuaos os upgrade --to 1.0.1 --reapply -y
vos vdm status
vos vdm install [--ha] [--bridge vmbr0] [--ipv4 dhcp|IP/CIDR]
vos vdm update
vos vdm movenode <target-node>
vos vdm uninstall
vos vdm ha enable|disable|status
vos docker ps
vos docker logs <id> [--tail N]
vos docker stats <id>
vos docker exec <id> <command...>
vos docker start|stop|restart <id>
vos docker prune [all|containers|images|volumes|networks]
vos services
vos services restart [unit]
vos services status <unit>
```

`virtuaos` and `vos` are identical. VDM installation performs cluster-wide
discovery and locking: it refuses while another node reports an existing VDM,
a VDM registration, an installation lock, or cannot be verified.

`vos docker` talks directly to the local runner socket
(`/run/auxinuxvirtual.sock`), so it works on the host without the Node CLI and
without an API session. `vos services` lists and restarts the Virtua stack
systemd units.

`virtuaos version` now prints the VirtuaOS release marker from
`/etc/virtuaos-version` when present, so `vos os upgrade` becomes visible in the
regular version output too.

See `VirtuaOS/README-OSUPGRADE.md` for the full instruction file format.

`virtuaos os ...` only works on VirtuaOS. It reads the installed OS release from
`/etc/virtuaos-version` first, then falls back to `/etc/os-release`. The public
release manifest lives at `https://dep.auxinux.ca/VIRTUAOS/release.md`; upgrades
are driven by `https://dep.auxinux.ca/VIRTUAOS/<version>/upgrade.instruction`.

## Build

```
cargo build --release    # -> target/release/virtuaos
```

The shipped artifact targets **linux/amd64**. On a non-amd64 dev box, build inside
the Debian build container or cross-compile (`--target x86_64-unknown-linux-gnu`).

## Packaging (Phase 3)

Ships as the Debian package `virtuaos-cli` in the `virtua` suite of
`dep.auxinux.ca/VIRTUA`, installs `/usr/bin/virtuaos`, and self-updates via
`virtuaos update`. The `auxinux-virtua` package supplies the matching
`/opt/auxinux-virtua/INSTALL/vdm-install.sh` payload.
