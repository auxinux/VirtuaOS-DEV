#!/usr/bin/env bash
# =============================================================================
#  VirtuaOS live-build configuration
#  - Pure live system (no debian-installer)
#  - Custom installer launched from live boot
#  - Everything pre-built in chroot for OFFLINE installation
#
#  Kernel: instead of the stock Debian "linux-image-amd64", VirtuaOS ships the
#  AuxiNux hypervisor kernel 7.x-auxinux-virtua. This is wired through
#  live-build's kernel stage with:
#      --linux-packages "kernel"  --linux-flavours "virtua"
#  which schedules the meta-package "kernel-virtua" (== ${LB_LINUX_PACKAGES}-${FLAVOUR})
#  for installation. The meta pulls linux-image-<ver>-auxinux-virtua-amd64 +
#  matching headers, so the versioned vmlinuz/initrd land in /boot and the
#  binary stage (which globs chroot/boot/vmlinuz-*) embeds them into /live.
#  The kernel-virtua package is served by the AuxiNux APT repo configured in
#  config/archives/auxinux-kernel.list.chroot (key injected by build.sh).
# =============================================================================

lb config \
    --distribution "trixie" \
    --archive-areas "main contrib non-free non-free-firmware" \
    --architecture "amd64" \
    --binary-images "iso-hybrid" \
    --bootloaders "grub-pc,grub-efi" \
    --debian-installer "none" \
    --apt-indices "false" \
    --apt-recommends "false" \
    --apt-secure "true" \
    --debootstrap-options "--include=ca-certificates,locales" \
    --bootappend-live "boot=live components quiet splash username=virtua hostname=virtuaos-live noeject persistence-encryption=none" \
    --iso-application "VirtuaOS" \
    --iso-preparer "VirtuaOS Build System" \
    --iso-publisher "AuxiNux — https://auxinux.io" \
    --iso-volume "VirtuaOS-1.0.3-BETA" \
    --memtest "none" \
    --win32-loader "false" \
    --linux-packages "kernel" \
    --linux-flavours "virtua" \
    --image-name "VirtuaOS-1.0.3-BETA-amd64" \
    --checksums "sha256" \
    --compression "xz" \
    --firmware-binary "true" \
    --firmware-chroot "true" \
    --mirror-bootstrap "http://deb.debian.org/debian/" \
    --mirror-chroot "http://deb.debian.org/debian/" \
    --mirror-chroot-security "http://security.debian.org/debian-security" \
    --mirror-binary "http://deb.debian.org/debian/" \
    --mirror-binary-security "http://security.debian.org/debian-security"
