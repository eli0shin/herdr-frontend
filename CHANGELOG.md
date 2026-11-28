# herdr-frontend

## 0.1.1

### Patch Changes

- [#52](https://github.com/eli0shin/herdr-frontend/pull/52) [`af9a6cf`](https://github.com/eli0shin/herdr-frontend/commit/af9a6cff185006e92ec4412aa71114d8f77f3b9c) Thanks [@eli0shin](https://github.com/eli0shin)! - The macOS download is for Apple silicon only: the app is arm64, with no Intel build. Notarized releases now staple the notary ticket to the app rather than its zip, so the release job no longer fails after notarization.

## 0.1.0

### Minor Changes

- [#51](https://github.com/eli0shin/herdr-frontend/pull/51) [`378f17c`](https://github.com/eli0shin/herdr-frontend/commit/378f17c88a02cb7ad0e2b6c4002c3bb58c295fb4) Thanks [@eli0shin](https://github.com/eli0shin)! - Each release has a herdr-frontend .dmg for Apple silicon and Intel with its SHA-256, signed and notarized once the release secrets are set, and the app carries its license and third-party notices.

- [#49](https://github.com/eli0shin/herdr-frontend/pull/49) [`cbc2e1d`](https://github.com/eli0shin/herdr-frontend/commit/cbc2e1dcccf23b19e68eb024ca1de33739de41f5) Thanks [@eli0shin](https://github.com/eli0shin)! - Versioned releases: each release is tagged and published as a GitHub Release with its changelog, and `herdr-frontend --version` and the app bundle show the released version.
