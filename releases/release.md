# VirtuaOS releases

latest: 1.0.3

## 1.0.0

Initial public VirtuaOS release marker. This version writes
`/etc/virtuaos-version` as `1.0.0` and keeps the base AuxiNux packages aligned
with the public repositories.

## 1.0.1

Maintenance release for the release channel. This one updates the release
marker to `1.0.1`, refreshes the branding files, and does not require a fresh
install.

## 1.0.3

Release aligning the displayed version everywhere (login banner, web panel and
`vos version` now read the same `/etc/virtuaos-version` marker), shipping the
AuxiNux kernel 7.1.10 and the Virtua panel 0.7.48, and adding a hardware
summary (CPU / RAM / disk / GPU / KVM) to `vos status`. The login banner and
`virtuaos-status` are now dynamic and follow upgrades automatically.
