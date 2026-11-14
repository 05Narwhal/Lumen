# Security policy

## Reporting a vulnerability

Please **do not open a public issue**. Use GitHub's private vulnerability reporting: on the repository, open *Security* → *Report a vulnerability*. You will get a reply and an honest timeline. Lumen is maintained by volunteers, so please allow some days before you expect a fix.

Useful in a report: the Lumen version (Settings → About), your operating system, the steps, and what an attacker gains. Please never attach real photos or a real library.

## What is in scope

- Lumen's code in this repository, including the updater, the installer (*Lumen Setup*), the local MCP server and the `photos://` protocol.
- **The plugin sandbox.** A plugin is a WebAssembly component run by `photos-plugins` (wasmtime) with a 512 MB memory limit, a 30 s time limit and three traps before it is stopped, and with no filesystem, network or process access. A way for a plugin to read or write outside what its interface gives it, to exceed those limits without being stopped, or to forge a signature or licence check is a vulnerability.
- Anything that could modify, delete or leak a person's originals or library.

## What is out of scope

- Findings that need a malicious local user who already controls the person's account or computer.
- Third-party software Lumen starts or loads (FFmpeg, ExifTool, Adobe DNG Converter, the person's own editors). Report those upstream; we will update the version we ship.
- Plugins' own behaviour inside what the sandbox allows, and what a plugin's metadata claims.

## Supported versions

Lumen versions are `26.X.Y-<channel>`. Only the **latest release** of each channel (alpha, beta, stable) gets security fixes, and a fix ships as a new release on that channel; the app's updater offers it.

## How updates are protected

Updates are signed with a key whose public half is in the app; an update that does not verify is refused. Plugins are checked against a signature and a declared licence before they are installed.
