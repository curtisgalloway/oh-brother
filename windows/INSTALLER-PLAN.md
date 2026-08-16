<!--
SPDX-FileCopyrightText: 2026 Curtis Galloway
SPDX-License-Identifier: Apache-2.0
-->

# Windows installer plan: WiX v7 MSI + Burn bundle

Research and implementation notes, August 2026. This builds on the earlier
format evaluation (MSI-in-Burn over MSIX/NSIS/zip) and turns it into working
authoring: `installer/Package.wxs`, `installer/Bundle.wxs`, the `msi` and
`bundle` actions in `build.ps1`, and `.github/workflows/release-windows.yml`.
The goal driving every choice here: strangers can install with two clicks and
update with one command, without Curtis running a signing bureaucracy.

## What was verified about the toolchain (August 2026)

WiX v7.0.0 is the current stable release, shipped April 6, 2026, following
v6 (April 2025); v3–v5 are out of community support. v7 keeps the SDK-style
project format and the `wix` .NET CLI tool, so nothing about this authoring
is version-precarious — it targets the v4 schema namespace that v5/v6/v7 all
compile. The colleague's "WiX v7" call checks out.

WiX's Open Source Maintenance Fee applies to "consumers of the WiX Toolset
project who generate revenue." oh-brother is Apache-2.0 hobby software with
no revenue, so the fee does not apply; nothing to pay, nothing to configure.

The MSIX rejection stands and got stronger: beyond the PATH-write
virtualization problem already identified, MSIX cannot ship unsigned at all,
while this plan's signing story (below) is Authenticode-based and optional
to bootstrap.

## The shape of the thing

Two artifacts per release, built from the same MSI:

**`OhBrother-<version>-x64.msi`** — a per-user MSI (`Scope="perUser"`,
WiX v5+ attribute) installing to `%LOCALAPPDATA%\Programs\Oh Brother`, the
exact folder the old xcopy install used. No UAC prompt, a real Add/Remove
Programs entry with the app icon, `MajorUpgrade` so any newer MSI installed
over an older one upgrades in place, and a Start Menu shortcut authored as a
proper MSI shortcut rather than a WScript hack. Every component keypaths on
an HKCU registry value under `Software\oh-brother\msi`, which is what
per-user file components require to validate cleanly (ICE38). This is the
artifact winget distributes.

**`OhBrother-<version>-x64-setup.exe`** — a Burn bundle wrapping that MSI
for the "clicked a download link" audience. Its one job beyond the MSI is
chaining the WebView2 Evergreen Bootstrapper, which the wry shell needs at
run time. Two registry searches (HKLM per-machine, HKCU per-user, EdgeUpdate
client id `{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`) detect an existing
runtime, and since Win11 and updated Win10 ship WebView2, the chain step is
skipped on nearly every machine. When it does run, it downloads the runtime
from Microsoft and elevates (WebView2's installer is per-machine); the
package is marked `Permanent` so uninstalling Oh Brother never rips out a
shared runtime. `build.ps1` downloads the ~2 MB bootstrapper stub fresh at
build time and compresses it into the bundle — Microsoft revs the stub
server-side, so pinning a hash of a remote payload would rot.

The app's Tools → "Install 'label' Command in PATH" feature needs no
changes: under plain MSI its winreg PATH edit and `label.cmd` shim work
exactly as written (the thing MSIX would have broken). The MSI deliberately
does not duplicate that feature. Two consequences to accept for now: the
shim survives uninstall as orphaned app data, and the fonts cache in
`%LOCALAPPDATA%\oh-brother` does too. Both are candidates for a later
`WixRemoveFolderEx`-style cleanup pass if it starts to matter.

## The update story

For winget users, updates are `winget upgrade` (or automatic, for people who
enable that): the release workflow's last job PRs each new version's MSI
into `microsoft/winget-pkgs` via the winget-releaser action, and MSI
`MajorUpgrade` makes the install-over-install clean. For direct downloaders,
running a newer setup.exe or MSI over an old install upgrades in place —
Burn registers the bundle with its own `UpgradeCode` so related bundles
supersede each other the same way MSIs do.

Two winget specifics worth knowing. First, a package's *first* version
cannot be submitted by the action — create it once by hand with
`wingetcreate new` (or komac) against the v-tagged release's MSI URL, under
the identifier `CurtisGalloway.OhBrother`; the action then handles every
subsequent version. Second, author that first manifest with `Scope: user`
and let wingetcreate capture the ProductCode — winget has historically had
upgrade-detection quirks with per-user MSIs when the manifest is vague about
scope (winget-cli #3011).

An in-app "check for updates" (hit the GitHub releases API, offer the new
bundle) would be a nice third leg someday, but winget + MajorUpgrade already
delivers the "easily update" goal without writing updater code.

## Code signing: SignPath Foundation

Unsigned installers work but greet every downloader with a SmartScreen
"unrecognized app" wall — the single biggest friction for the "easy for
others to install" goal. The decision (made in-session): apply to SignPath
Foundation's free OSS signing program rather than pay for a certificate.

What was verified about the program: it's free for open-source projects
meeting their conditions — OSI license without commercial dual-licensing, no
proprietary components, active maintenance, binaries produced by an
automated CI build from the public repo, MFA for the team, and a published
code-signing policy with named author/reviewer/approver roles (a solo
maintainer can hold multiple roles). The certificate names **SignPath
Foundation** as publisher, not Curtis — that's the trade for free. The
for-pay alternative if that ever chafes: Azure Artifact Signing (the renamed
Trusted Signing), now GA at $9.99/month and open to self-employed
individuals in the US with identity verification — cert in your own name
and near-instant SmartScreen reputation.

Signing a Burn bundle is a three-signature dance, and the workflow encodes
it: sign the MSI; build the bundle around the signed MSI; `wix burn detach`
the Burn engine and sign it; `wix burn reattach`; sign the finished bundle.
With SignPath that's three signing-request round-trips per release, all
automated once onboarded. Until onboarding completes, every signing step in
the workflow is gated on `vars.SIGNPATH_ORGANIZATION_ID` and simply skips —
releases ship unsigned but everything else works, so nothing blocks on the
SignPath application.

## What's in this change

`installer/Package.wxs` (the MSI), `installer/Bundle.wxs` (the bundle),
`build.ps1` grew `msi` and `bundle` actions plus a `-NoCargo` switch and now
reads the version from the workspace `Cargo.toml` (single source of truth —
release = bump that version, tag `v<version>`, push), and the release
workflow runs the whole pipeline on tag push — note it is **staged at
`installer/release-windows.yml`** because remote tooling cannot write into
`.github/workflows/`; `git mv` it to `.github/workflows/release-windows.yml`
before the first tagged release. The pipeline:
cargo build → MSI → (sign) → bundle → (sign engine + bundle) → GitHub
Release → winget PR. Both UpgradeCodes were generated once and are now
permanent constants — never change `8B1FB345-683B-4651-98F8-F7A3DD8E509E`
(MSI) or `EAADD724-5F7C-4B78-AA73-5DCD3C8CD849` (bundle), or upgrades break.

Caveats in the same spirit as TESTING.md: none of this has executed on a
real Windows machine. The `.wxs` files are well-formed XML and were authored
against the v4-schema documentation, but `wix build` has not run over them
(this session's sandbox can't reach NuGet to install the WiX CLI), and the
SignPath action's input names were written from its v1.2 docs without a live
run. Expect a first-run fix-forward pass, same as the rest of the Windows
port.

## One-time setup remaining (in order, none blocking the next)

1. ~~Verify the plain build~~ PARTIALLY DONE 2026-08-15 on brik: WiX
   OSMF EULA accepted (user-approved), `build.ps1 msi` and
   `build.ps1 bundle` both build (after fixing WIX0230 — the three
   file+registry-keypath components needed explicit permanent guids),
   and the MSI passed a silent install/uninstall round-trip (files +
   Start Menu shortcut appear and are removed cleanly; the product
   registers in the per-user Installer database). ARPURLINFOABOUT is
   fixed. Remaining: eyeball the Add/Remove Programs entry in
   Settings (no HKCU Uninstall key was visible over SSH — confirm the
   entry, icon, and version render in Settings ▸ Apps), launch the
   installed app from the Start Menu, and run the bundle on a
   machine/VM without WebView2.
2. ~~git mv the workflow into .github/workflows/~~ (done 2026-08-15,
   plus a WiX OSMF EULA acceptance step — `wix eula accept wix7` —
   which the first real `wix build` revealed is mandatory, error
   WIX7015; a macOS DMG workflow landed alongside it). Remaining: tag
   `v0.1.0` and let both workflows produce an unsigned release
   end-to-end.
3. Apply at signpath.org: publish a short code-signing policy page in the
   repo, enable MFA, reference the release workflow. On approval, set
   `SIGNPATH_API_TOKEN` (secret) + `SIGNPATH_ORGANIZATION_ID` (variable) and
   align the project/policy slugs in the workflow.
4. Submit the first winget manifest by hand with wingetcreate
   (`Scope: user`), then set `WINGET_TOKEN` (classic PAT, public_repo) so
   the action takes over from the second release on.

## Sources

- [WiX v7 release announcement (FireGiant)](https://www.firegiant.com/blog/2026/4/6/wix-v7-heatwave-and-heatwave-build-tools-are-released/) and [wixtoolset/wix releases](https://github.com/wixtoolset/wix/releases)
- [Open Source Maintenance Fee (wixtoolset issue #8974)](https://github.com/wixtoolset/issues/issues/8974)
- [WiX Package/@Scope perUser and INSTALLFOLDER (issue #8101)](https://github.com/wixtoolset/issues/issues/8101)
- [Signing packages and bundles (FireGiant docs)](https://docs.firegiant.com/wix/tools/signing/)
- [SignPath Foundation conditions](https://signpath.org/terms.html) and [SignPath OSS program](https://signpath.io/solutions/open-source-community)
- [Azure Artifact Signing for individuals (Melatonin write-up)](https://melatonin.dev/blog/code-signing-on-windows-with-azure-trusted-signing/), [pricing/GA coverage (DevClass)](https://www.devclass.com/security/2026/01/14/code-signing-windows-apps-may-be-easier-and-more-secure-with-new-azure-artifact-service/4079554)
- [winget-releaser action](https://github.com/vedantmgoyal9/winget-releaser), [komac](https://github.com/russellbanks/Komac), [winget per-user MSI scope quirk (winget-cli #3011)](https://github.com/microsoft/winget-cli/issues/3011)
- [WebView2 distribution / Evergreen Bootstrapper link](https://go.microsoft.com/fwlink/p/?LinkId=2124703)
