# VirtuaOS OS Upgrade Format

`vos os upgrade` reads a JSON file named `upgrade.instruction` from:

```text
https://dep.auxinux.ca/VIRTUAOS/<version>/upgrade.instruction
```

The file describes how to transform an installed VirtuaOS host into that
target release without reinstalling from scratch.

## Rules

1. The upgrade runs as root.
2. The engine only runs on VirtuaOS.
3. The engine applies steps in a fixed order.
4. The public release manifest is:
   `https://dep.auxinux.ca/VIRTUAOS/release.md`
5. The installed VirtuaOS version marker is read from:
   `/etc/virtuaos-version`
6. `vos os upgrade --reapply` can replay the same release to refresh branding
   or files without changing the release number.

## JSON Shape

```json
{
  "version": "1.0.1",
  "messages": ["..."],
  "aptUpdate": true,
  "aptFullUpgrade": false,
  "aptInstall": ["pkg1", "pkg2"],
  "aptRemove": ["pkg3"],
  "files": [
    {
      "url": "https://dep.auxinux.ca/VIRTUAOS/1.0.1/path/to/file",
      "path": "/etc/example.conf",
      "mode": "0644"
    }
  ],
  "downloads": [
    {
      "url": "https://dep.auxinux.ca/VIRTUAOS/1.0.1/assets/blob.bin",
      "path": "/var/lib/example/blob.bin"
    }
  ],
  "downloadedScripts": [
    {
      "url": "https://dep.auxinux.ca/VIRTUAOS/1.0.1/scripts/post-upgrade.sh",
      "path": "/usr/local/sbin/post-upgrade.sh",
      "args": ["--apply"],
      "cwd": "/",
      "mode": "0755",
      "description": "Run post-upgrade script"
    }
  ],
  "commands": [
    {
      "description": "Set version marker",
      "run": "printf '%s\\n' '1.0.1' > /etc/virtuaos-version"
    }
  ],
  "rebootRequired": false
}
```

## Fields

### `version`

Required. Must match the target folder name. Example:

```json
"version": "1.0.1"
```

The engine refuses to continue if the JSON says a different version than the
requested release.

### `messages`

Optional array of human-readable status lines. They are printed before any
action starts.

Use this for context only. Do not put secrets here.

### `aptUpdate`

Optional boolean.

When `true`, the engine runs:

```bash
apt-get update
```

Use this when the release depends on fresh package indexes.

### `aptFullUpgrade`

Optional boolean.

When `true`, the engine runs:

```bash
apt-get full-upgrade -y
```

Use this only when the release needs package replacements or dependency
reshuffling, not for every maintenance bump.

### `aptInstall`

Optional array of package names.

Each entry is installed with:

```bash
apt-get install -y <package...>
```

Example:

```json
"aptInstall": ["kernel-virtua", "auxinux-virtua", "virtuaos-cli"]
```

Use this for packages that must exist after the upgrade.

### `aptRemove`

Optional array of package names.

Each entry is removed with:

```bash
apt-get remove -y <package...>
```

Use this to retire old packages that should no longer remain installed.

### `files`

Optional array of file copy instructions.

Each item downloads a file, stores it in a temporary workdir, copies it to the
final `path`, then applies `mode` if provided.

Fields:

- `url`: direct download URL. If omitted, the engine uses
  `https://dep.auxinux.ca/VIRTUAOS/<version>/<path>`
- `path`: destination path on the host
- `dest`: alias of `path` for compatibility
- `mode`: optional file mode like `0644` or `0755`

Use this for configs, service files, shell scripts that will be executed later,
or any other payload that should exist on disk before another step uses it.

Example:

```json
"files": [
  {
    "url": "https://dep.auxinux.ca/VIRTUAOS/1.0.1/etc/virtuaos-banner",
    "path": "/etc/virtuaos-banner",
    "mode": "0644"
  }
]
```

### `downloads`

Optional array with the same structure as `files`.

It exists as a compatibility alias. Use either `files` or `downloads`, not both
for the same object unless you intentionally want to process both lists.

### `downloadedScripts`

Optional array of scripts to download and execute.

This is the structured replacement for the common pattern:

1. download a script
2. chmod it executable
3. run it immediately

Fields:

- `url`: direct download URL
- `path`: optional local target path. If omitted, the script is downloaded into
  the work directory using the URL filename
- `args`: optional array of arguments passed to the script
- `cwd`: optional working directory used when running the script
- `mode`: optional file mode, default `0755`
- `description`: optional log line shown before execution

Example:

```json
"downloadedScripts": [
  {
    "url": "https://dep.auxinux.ca/VIRTUAOS/1.0.1/scripts/install-extra.sh",
    "path": "/usr/local/sbin/install-extra.sh",
    "args": ["--quiet"],
    "cwd": "/",
    "mode": "0755",
    "description": "Run release helper"
  }
]
```

This is the right field when a release needs to fetch a helper script and run
it as part of the upgrade.

### `commands`

Optional array of raw shell commands.

Each command runs through:

```bash
bash -lc "<run>"
```

Fields:

- `run`: shell command to execute
- `description`: optional log line shown before the command

Use this for small one-off adjustments such as writing a marker file, creating a
directory, or toggling a service flag.

Example:

```json
"commands": [
  {
    "description": "Set VirtuaOS release marker",
    "run": "printf '%s\\n' '1.0.1' > /etc/virtuaos-version"
  }
]
```

Avoid putting large scripts here. Prefer `downloadedScripts` for that.

### `rebootRequired`

Optional boolean.

When `true`, the engine prints a reboot reminder after the upgrade completes.

Use this for kernel, initramfs, bootloader, or low-level changes that only take
effect after reboot.

## Execution Order

The engine applies steps in this order:

1. `messages`
2. `aptUpdate`
3. `aptFullUpgrade`
4. `aptInstall`
5. `aptRemove`
6. `files`
7. `downloads`
8. `downloadedScripts`
9. `commands`
10. `rebootRequired` reminder

## Recommended Pattern

For a release that should behave like a real ISO upgrade, use this order:

1. install or remove packages with `aptInstall` and `aptRemove`
2. drop config files with `files`
3. execute helper scripts with `downloadedScripts`
4. finish with `commands` for tiny markers or cleanup
5. mark `rebootRequired` if the kernel or boot path changed

If you need to patch files in the same release after publication, update the
release assets on the server and use `vos os upgrade --to <version> --reapply`
on existing hosts.

## Minimal Example

```json
{
  "version": "1.0.2",
  "aptUpdate": true,
  "aptInstall": ["virtuaos-cli"],
  "files": [
    {
      "url": "https://dep.auxinux.ca/VIRTUAOS/1.0.2/etc/example.conf",
      "path": "/etc/example.conf",
      "mode": "0644"
    }
  ],
  "downloadedScripts": [
    {
      "url": "https://dep.auxinux.ca/VIRTUAOS/1.0.2/scripts/post-upgrade.sh",
      "path": "/usr/local/sbin/post-upgrade.sh",
      "mode": "0755"
    }
  ],
  "commands": [
    {
      "run": "printf '%s\\n' '1.0.2' > /etc/virtuaos-version"
    }
  ],
  "rebootRequired": false
}
```

## Practical Notes

- Keep `upgrade.instruction` small and explicit.
- Put reusable assets next to the version folder on the server.
- If the upgrade needs a package from a repository, make sure that repository is
  already enabled before the release is published.
- If a change is truly destructive or irreversible, do not hide it in a command
  blob. Make it explicit in the release notes first.
