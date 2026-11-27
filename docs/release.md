# Releases

herdr-frontend is versioned with [Changesets](https://changesets.dev), as Eli's other repos
(eli0shin/repos, eli0shin/code-review-tui) are: each user-visible PR adds a changeset, a bot PR
gathers them into a version bump and changelog, and merging that PR tags the version and creates
its GitHub Release. Everything here runs on free `ubuntu-latest` runners.

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
4. Merging the version PR leaves `main` with no changesets. The next `version.yml` run checks that
   the three files agree, then runs `changeset git-tag`, which tags `v<version>` (skipped when the
   tag exists). The action pushes the tag and creates the GitHub Release `v<version>`, its notes
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

`.github/workflows/release-artifacts.yml` is where a release's build and upload go (ticket 045). It
takes the release's `tag` and runs:

- from `version.yml`'s `artifacts` job, right after a release is created. A release created with
  `GITHUB_TOKEN` starts no `release` or tag `push` workflow, so this call is the only trigger;
- by hand, for a release that needs its artifacts again: Actions > release artifacts > Run
  workflow, with the tag.

It checks out the tag and checks that the release exists and that the tag's `package.json`,
`Cargo.toml` and `Cargo.lock` carry the tag's version. It builds and uploads nothing yet. Uploading
needs `contents: write`, both on the `artifacts` job in `version.yml` and in the workflow itself;
both say `contents: read` for now.

## Repository settings

`changesets/action` opens the version PR with `GITHUB_TOKEN`, so Settings > Actions > General >
Workflow permissions must have **Allow GitHub Actions to create and approve pull requests** on (as
Eli's other repos do). Without it, the run fails with "GitHub Actions is not permitted to create or
approve pull requests". Each job declares the token permissions it needs, so the default
workflow permissions can stay read-only.
