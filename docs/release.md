# Releases

herdr-frontend is versioned with [Changesets](https://changesets.dev), as Eli's other repos
(eli0shin/repos, eli0shin/code-review-tui) are: each user-visible PR adds a changeset, a bot PR
gathers them into a version bump and changelog, and merging that PR tags the version and creates
its GitHub Release, and one macOS job builds, signs, notarizes and uploads the release's .dmg.
Everything else runs on `ubuntu-latest`. All of it is free: standard GitHub-hosted runners,
macOS included, cost nothing in a public repository.

## The version

The version lives in the root `package.json`, a private package that exists only for versioning
(nothing is published to npm). Bun installs the Changesets CLI from `bun.lock`.
`scripts/sync-version` copies that version into the workspace `Cargo.toml`
(`[workspace.package] version`, which every crate inherits) and into `Cargo.lock`. It edits the
lock as text: cargo isn't needed, and every git dependency doesn't have to be fetched. From there
it reaches:

- `herdr-frontend --version`, which prints `herdr-frontend <version>`;
- `CARGO_PKG_VERSION` in every crate;
- the bundle's `Info.plist` (`CFBundleShortVersionString` and `CFBundleVersion`), which
  `scripts/bundle-macos` reads from `Cargo.toml`.

Never edit the version by hand. `just test` runs `scripts/sync-version --check`, which fails when
`package.json`, `Cargo.toml` and `Cargo.lock` disagree, and `crates/app/tests/version.rs` checks
that `--version` prints `package.json`'s version.

## Adding a changeset

Every user-visible PR adds a changeset: a change Eli would notice in the app, its CLI, its bundle
or its install. Run `bun install` once, then:

```sh
bunx changeset          # pick the bump and write the summary
```

or write `.changeset/<any-name>.md` by hand:

```md
---
"herdr-frontend": patch
---

Switching tabs keeps the scroll position of each pane.
```

The summary is one line in the release notes, written for Eli: what changed for him, not how. The
package is still 0.x, so use `minor` for a new feature or a change in behavior and `patch` for a
fix. `major` (1.0.0) is Eli's call.

The `changeset check` workflow runs `changeset status` on each PR. It fails when files matching
`changedFilePatterns` in `.changeset/config.json` changed and the PR added no changeset. Those
files are the crates' sources, build scripts and manifests, `Cargo.toml`, `Cargo.lock`,
`rust-toolchain.toml`, `assets/`, `scripts/bundle-macos` and `scripts/install-macos`. Tests, docs
and the other scripts don't count. A PR that touches those files but changes nothing Eli would see
(a refactor, say) adds an empty changeset with `bunx changeset --empty`.

## The flow

1. A PR adds `.changeset/*.md` and merges to `main`.
2. `version.yml` runs on the push to `main`. With changesets present, `changesets/action` runs
   `bun run version-packages` (`changeset version`, then `scripts/sync-version`) on the
   `changeset-release/main` branch, and opens or updates the **"chore: version packages"** PR. That
   PR bumps `package.json`, `Cargo.toml` and `Cargo.lock`, writes `CHANGELOG.md` (with links to
   each PR and its author, from `@changesets/changelog-github`) and deletes the changesets. More
   changesets merged to `main` before it lands update the same PR.
3. A push or PR made with the workflow's `GITHUB_TOKEN` starts no workflows, so `version.yml` starts
   `ci` on `changeset-release/main` itself (`ci.yml` takes `workflow_dispatch`). The version PR's
   head commit gets its `ci` check like any other PR. The `changeset check` doesn't run on it.
4. Merging the version PR leaves `main` with no changesets. The `version.yml` run for that merge
   checks that the three files agree, then runs `changeset git-tag`, which tags `v<version>`
   (skipped when the tag exists). The action pushes the tag and creates the GitHub Release `v<version>`, its notes
   that version's `CHANGELOG.md` entry.

   A run only tags when `main` has no changesets. So merge the version PR after the run for the
   latest push to `main` has updated it. If a changeset lands on `main` first, the run after the
   merge opens a new version PR instead, and the merged version is never tagged.
5. The same run then calls `release-artifacts.yml` with the tag (below).

`version.yml` runs one at a time, each push's in turn (`queue: max`), so two pushes to `main`
can't race on the version branch or a tag. No run waiting is cancelled either: the run after a
version PR merges is the only one that tags that version, since a later run tags only the version
`package.json` has then.

## Artifacts

`.github/workflows/release-artifacts.yml` builds each release's macOS download and uploads it to
the release:

- `herdr-frontend-<version>-macos.dmg`: `herdr-frontend.app`, one universal binary for Apple
  silicon and Intel, beside an Applications link to drag it onto;
- `herdr-frontend-<version>-macos.dmg.sha256`, its SHA-256 (`shasum -a 256 -c` checks it).

It takes the release's `tag` and runs:

- from `version.yml`'s `artifacts` job, right after a release is created. A release created with
  `GITHUB_TOKEN` starts no `release` or tag `push` workflow, so this call is the only trigger.
  `version.yml` passes the signing secrets through (a called workflow gets none unless passed);
- by hand, for a release that needs its artifacts again (after setting the secrets, say):
  Actions > release artifacts > Run workflow with the tag, or
  `gh workflow run release-artifacts.yml -f tag=v<version>`. Uploads replace the release's
  existing files.

Its `check` job, on Linux, checks that the release exists and that the tag's `package.json`,
`Cargo.toml` and `Cargo.lock` carry the tag's version. Its `macos` job then runs
`scripts/release-macos` on a `macos-26` runner (Apple silicon), uploads the two files with
`gh release upload --clobber`, and writes an **Install** section into the release notes, between
`<!-- release-artifacts: install -->` markers that each run replaces. The section says whether the
download is notarized, and the job summary says the same. Both jobs need `contents: write` for the
upload, which `version.yml` grants the `artifacts` job.

### Where it builds

On a macOS runner, only for a release. Cross-compiling from Linux was the first choice and isn't
allowed:

- Linking the app needs the macOS SDK. Rune links AppKit, Metal, QuartzCore and CoreText, and the
  libghostty-vt xcframework is a macOS static library. Neither `cargo-zigbuild` nor osxcross
  carries Apple's frameworks: Zig ships no framework stubs, and osxcross packages an SDK taken out
  of Xcode. So either way the build needs Apple's SDK on a Linux machine.
- Apple's [Xcode and Apple SDKs Agreement](https://www.apple.com/legal/sla/docs/xcode.pdf)
  (EA2002, 2026-06-08) forbids that. Its opening note: "use of Apple software ... is authorized
  only for execution on an Apple-branded product running macOS." §2.2.A grants installing the
  Apple Software (which includes the macOS SDK) "on Apple-branded computers that are owned or
  controlled by You". §2.5 expressly prohibits "separately using the Apple SDKs or attempting to run
  any part of the Apple Software on non-Apple-branded hardware". §2.7: you agree not to "install,
  use or run the Apple Software ... on any non-Apple-branded computer or device".
- Signing and notarization could run on Linux (`rcodesign`), but they'd only help if the build
  could.

So one job runs on GitHub's macOS runner, which is Apple hardware, and only from this workflow,
never on a PR or a push. Standard runners, macOS included, are free and unlimited in public
repositories ([GitHub Actions billing](https://docs.github.com/en/billing/concepts/product-billing/github-actions)),
so a release costs nothing. If the repository goes private, each release's macOS minutes draw on
the account's included Actions usage. `ci.yml` stays on Linux.

### What it builds

`scripts/release-macos [<out-dir>]` (default `target/dist`; `just package` runs it):

1. Sets up signing (below) before building, so bad secrets fail in a minute, not after the build.
2. `scripts/bundle-macos --universal`: release builds for `aarch64-apple-darwin` and
   `x86_64-apple-darwin` (it adds the rustup targets), joined with `lipo` into
   `target/universal-apple-darwin/release/herdr-frontend.app`. Intel is cheap: the
   libghostty-vt xcframework's macOS slice is already universal, and `ghostty-vt-sys` thins it to
   each target. It costs a second build, about 4.5 minutes here (fat LTO); the final link peaks at
   2.6 GB, well inside the runner's 7 GB.
3. Signs the app (below), with the hardened runtime and `assets/macos/herdr-frontend.entitlements`.
4. With the Developer ID: zips the app with `ditto`, notarizes the zip, staples the ticket to the
   app, and checks it with `spctl --assess --type execute`.
5. Builds the .dmg with `hdiutil` (UDZO, compressed), signs it, notarizes and staples it, and checks
   it with `spctl --assess --type open`.
6. Mounts the .dmg read-only and checks what a download gets: the Applications link, the app's
   signature, both architectures, the license files, the stapled ticket when notarized, and that
   the app's own `--version` reports the version being released.
7. Writes the SHA-256 file and prints both paths. On Actions it also writes `dmg`, `sha256` and
   `signing` to the step's outputs.

The `herdr-frontend` command is the app's own binary, `Contents/MacOS/herdr-frontend`: the window,
`--version` and the `ls | do | status` CLI (`docs/cli.md`). The release notes give the one link to
make for it, as `just install` does.

**Why a .dmg, not a .zip.** A .dmg can be signed, notarized and stapled itself, so Gatekeeper
checks the download offline before it opens; a .zip can't carry a ticket, only the app inside it
can. The app inside is stapled too, so it keeps passing once copied out. It's also the usual Mac
download (drag to Applications), and what eli0shin/macparakeet ships.

`scripts/bundle-macos` (so `just bundle` and `just install` too) copies `LICENSE`,
`THIRD-PARTY-NOTICES.md` and every local file the notices link to into `Contents/Resources`, at
the same relative paths, so the notices' links resolve in the bundle. It skips the licenses of
files only the tests use, and fails if a linked file is missing.

### Signing

`scripts/release-macos` follows eli0shin/macparakeet's release (`publish_signed_artifact.sh` and
`sign_notarize.sh`) and its secret names. These are repository secrets (Settings > Secrets and
variables > Actions):

| Secret | What |
| --- | --- |
| `DEVELOPMENT_ID_CERTIFICATE_BASE64` | The Developer ID Application certificate and its private key, a .p12, base64 |
| `DEVELOPMENT_ID_CERTIFICATE_PASSWORD` | The .p12's password |
| `DEVELOPER_ID_APPLICATION_IDENTITY` | `Developer ID Application: <name> (<team ID>)` |
| `APPLE_TEAM_ID` | The 10-character team ID |
| `NOTARY_APPLE_ID` | The Apple ID notarytool signs in with |
| `NOTARY_APP_SPECIFIC_PASSWORD` | An app-specific password for that Apple ID |

- **All six set:** the .p12 is imported into a temporary keychain with a random password, which
  joins the keychain search list until the script exits, then is deleted (and the search list put
  back). The identity must be valid there. `notarytool store-credentials` checks the Apple ID,
  team and password into the same keychain. The app is signed with `--options runtime --timestamp`
  and the entitlements, and must show the hardened runtime, the identity as `Authority` and the
  team as `TeamIdentifier`. Each notarization is `notarytool submit --wait --timeout 1h`, and its
  log is printed (rejections list their problems there, acceptances their warnings). notarytool
  sometimes crashes mid-upload (SIGBUS) without a verdict, so a submission with no verdict is
  retried once. `Invalid` or `Rejected` fails the job.
- **None set:** the app is signed ad hoc, still with the hardened runtime, so it runs exactly as
  the notarized one does, and the .dmg isn't signed. Gatekeeper stops a downloaded copy until it's allowed
  in System Settings > Privacy & Security, or its quarantine attribute is removed. The release
  notes' Install section says, in bold, that it isn't notarized and how to open it, and the job
  summary and a warning annotation say it too.
- **Some set:** the job fails and names the missing ones, rather than ship something other than
  what was configured.

**Entitlements:** none. The entitlements file is an empty dictionary, explained in its comment: the
app uses no JIT or unsigned executable memory, no DYLD variables, no plug-ins or other teams'
libraries, no camera, microphone, location or Apple Events. Metal, CoreText and AppKit work under
the hardened runtime as they are, and the processes it starts (`hc`, `open`, link openers) aren't
limited by it. Terminal programs run under herdr-core, not this process, so nothing they do needs
an entitlement here. The ad hoc build checked below runs under the hardened runtime and renders
through Metal and CoreText.

### Setting the secrets

`scripts/bootstrap-release-secrets` sets all six on eli0shin/herdr-frontend with `gh secret set`:

```sh
scripts/bootstrap-release-secrets              # export the identity from your login keychain
scripts/bootstrap-release-secrets cert.p12     # or use a .p12 you exported
```

Without a .p12 it lists the valid Developer ID Application identities in your keychains, exports
the one you pick with its private key (macOS asks you to allow the export), and asks for a
password for it (empty generates one). `security export` can only export every identity in a
keychain, so the script exports just that one through the Security framework (`SecItemExport`).
It then checks the .p12 opens with its password and holds exactly one private key and a
Developer ID Application certificate, and asks for:

- **Team ID**, defaulting to the one in the certificate's name: developer.apple.com/account >
  Membership details;
- **Apple ID**: the email you sign in to developer.apple.com with;
- **App-specific password**: account.apple.com (formerly appleid.apple.com) > Sign-In and Security >
  App-Specific Passwords. Revoking it there cuts off the release job.

It shows what it's about to set and asks before setting anything. Everything stays in memory: it
writes no file, and the values reach `gh` on stdin, never on a command line. It's safe to run again
(a renewed certificate, a new app-specific password): it sets all six again. It doesn't check the
Apple ID and password itself; the next release run does that first, in its first minute.

### Checking it without the secrets

- `just package` runs `scripts/release-macos` with no secrets: the universal build, the ad hoc
  signature with the hardened runtime, the .dmg and its checks, as the release job runs them.
- `just package-dry-run` (`scripts/release-signing-dry-run`) runs the Developer ID path with a
  throwaway self-signed "Developer ID Application: herdr-frontend dry run (DRYRUN0000)" certificate,
  passed through the same six variables. It covers the temporary keychain, the import, the
  identity check, signing the app and the .dmg with a secure timestamp, and putting the search
  list back. It skips notarization (`RELEASE_SKIP_NOTARIZE=1`), since the notary service only takes
  a real Developer ID, then checks the signatures' authority, timestamp and hardened runtime.

What only a real run shows: the Developer ID chain on the runner, `notarytool`'s credentials check,
notarization and stapling, and `spctl`'s verdict.

### Left for Eli

1. Run `scripts/bootstrap-release-secrets` once and pick the Developer ID Application identity
   macparakeet signs with; it's in your login keychain. Make an app-specific password for
   herdr-frontend when it asks.
2. Merge the version PR. The release's artifacts job builds, notarizes and uploads the .dmg; its
   summary says whether it was notarized. A release made before the secrets were set can be
   redone with `gh workflow run release-artifacts.yml -f tag=v<version>`.

No Actions budget is needed while the repository is public.

## Repository settings

`changesets/action` opens the version PR with `GITHUB_TOKEN`, so Settings > Actions > General >
Workflow permissions must have **Allow GitHub Actions to create and approve pull requests** on (as
Eli's other repos do). Without it, the run fails with "GitHub Actions is not permitted to create or
approve pull requests". Each job declares the token permissions it needs, so the default
workflow permissions can stay read-only.
