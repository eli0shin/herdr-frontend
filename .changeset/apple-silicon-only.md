---
"herdr-frontend": patch
---

The macOS download is for Apple silicon only: the app is arm64, with no Intel build. Notarized releases now staple the notary ticket to the app rather than its zip, so the release job no longer fails after notarization.
