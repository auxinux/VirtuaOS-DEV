# VirtuaOS — turnkey hypervisor ISO

VirtuaOS is a bootable **amd64** installer ISO based on **Debian 13 (Trixie)**,
shipping the **AuxiNux hypervisor kernel `7.x-auxinux-virtua`** and the AuxiNux
Virtua control panel, pre-built for offline installation.

```
./build.sh            # build via Docker (macOS / any host with Docker)
sudo ./build.sh --native   # build natively on Debian/Ubuntu (needs live-build)
```

Output: `VirtuaOS-<version>-amd64.iso`. After install + first boot the panel is at
`http://<SERVER-IP>:8441` (`admin` / `admin123`).

## Separate repositories

VirtuaOS and the Virtua controller are separate projects. The default local
layout is:

```text
workspace/
├── Virtua/
└── VirtuaOS/
```

If the Virtua clone is elsewhere, provide it explicitly:

```bash
VIRTUA_SOURCE_DIR=/path/to/Virtua ./build.sh
```

The kernel source is maintained independently in `Kernel/VirtuaOS`; the ISO
consumes its published `kernel-virtua` package through the signed APT
repository.

## Custom kernel integration

VirtuaOS does **not** use the stock `linux-image-amd64`. The AuxiNux kernel is
wired into the live-build pipeline:

| Piece | Role |
|-------|------|
| `livecd/config.sh` → `--linux-packages "kernel" --linux-flavours "virtua"` | makes live-build install the meta-package **`kernel-virtua`** (`= ${LB_LINUX_PACKAGES}-${FLAVOUR}`), which pulls `linux-image-<ver>-auxinux-virtua-amd64` + headers |
| `livecd/package-lists/00-base.list.chroot` | `linux-image-amd64` removed so only the AuxiNux kernel is embedded |
| `livecd/archives/auxinux-kernel.list.chroot` | build-time APT source for `https://dep.auxinux.ca/VIRTUA kernel main` (removed from the final image by live-build) |
| `livecd/includes.chroot/etc/apt/sources.list.d/auxinux-kernel.sources` | same repo, **persists** on the installed system for future `apt full-upgrade` of the kernel |
| `build.sh` → `provision_kernel_repo_key()` | injects the repo signing key (build-time `.key.chroot` + persistent `.asc`) |
| `livecd/hooks/normal/9100-live-kernel-symlinks.hook.binary` | creates generic `/live/vmlinuz` + `/live/initrd.img` (live-build only emits versioned names; the static GRUB menus reference the generic paths) |

The kernel itself is built separately (`../Kernel/VirtuaOS/build-virtua-kernel.sh`) and
published to the APT repo (`../Kernel/VirtuaOS/build-depot-kernel.sh`). The ISO build
pulls `kernel-virtua` from that repo at build time and bakes it into the
squashfs, so the live environment **and** the installed system both run it.

### Repo signing key

By default `build.sh` downloads the armored key from
`https://dep.auxinux.ca/VIRTUA/virtua-archive-keyring.asc`.

While the public repo is not yet published (e.g. you are still building the
kernel), point the build at a local key instead — the one produced by
`build-depot-kernel.sh`:

```
AUXINUX_KEYRING_FILE=/var/www/dep.auxinux.ca/VIRTUA/virtua-archive-keyring.asc ./build.sh
```

You still need the repo itself reachable for `apt` to fetch `kernel-virtua`
during the chroot stage (a local/staging mirror works via `AUXINUX_KEYRING_URL`
+ editing `archives/auxinux-kernel.list.chroot`).
