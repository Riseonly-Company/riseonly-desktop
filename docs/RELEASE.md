# Build and release

Branches: `development` -> `staging` -> `main`. Everything goes through pull requests.

## Labels

| Label | Who sets it | What it does |
|---|---|---|
| `build` | ci, on every pull request into `staging` or `main` | builds the picked platforms and uploads them as workflow artifacts (7 days) |
| `deploy` | you | after the merge the build is released; without it a merge ships nothing |
| `macos` `linux` `win` | you | pick platforms; no platform label means all three |
| `tests` | you | also runs `cargo test` and clippy on Linux |

Any other pull request builds only when you add `build` yourself. Removing `build` stops builds on
later pushes. The labels are created by ci on the first pull request.

## Flows

- **pull request** (`build.yml`): lint (fmt, boundaries, module tree, canon locales) and a `fast`
  profile build of each picked platform. Into `main` it is a production app, otherwise a staging app.
- **development -> staging, merged with `deploy`** (`release.yml`): release profile build of the
  staging app (`Riseonly Staging`, talks to the staging backend), uploaded to the cdn under
  `staging/`, the staging feed updated, a draft GitHub release.
- **staging -> main, merged with `deploy`**: the same for production: the production feed, a
  published GitHub release with tag `v<version>`. It fails early when that tag exists, so bump
  `[workspace.package] version` in `Cargo.toml` first. The updater compares versions, so a staging
  build reaches staging installs only with a higher version too.

Any other merge into `staging` or `main` (another head branch, no `deploy`) releases nothing.

## Where the files go

```
<DESKTOP_DOWNLOADS_BASE>/<environment>/<version>-<build>/<Slug>-<version>-macos-aarch64.zip   updater
<DESKTOP_DOWNLOADS_BASE>/<environment>/<version>-<build>/<Slug>-<version>-macos-aarch64.dmg   first install
<DESKTOP_DOWNLOADS_BASE>/<environment>/<version>-<build>/<Slug>-<version>-linux-x86_64.AppImage
<DESKTOP_DOWNLOADS_BASE>/<environment>/<version>-<build>/<Slug>-<version>-windows-x86_64.zip
<DESKTOP_DOWNLOADS_BASE>/<environment>/feed.json       the feed rise_platform::updater::UpdateManifest reads
<DESKTOP_DOWNLOADS_BASE>/<environment>/feed.json.sig   base64 ed25519 signature of feed.json
```

`feed.json` keeps the last 12 versions. Each release carries `artifacts` (what the updater installs,
per target) and `downloads` (every file, the dmg included, for the site). A release with fewer
platforms keeps the other targets of the same version.

The publish step also posts the release as json to `BACKEND_RELEASE_HOOK` when that secret is set.
A failed hook is a warning, the feed is already out.

## Machines

- Linux and Windows build on the ci builder, in docker (`scripts/ci/toolchain/Dockerfile`, Ubuntu
  22.04 for the glibc floor, llvm 19, cargo-xwin for msvc). One runner per platform so both build
  at once:

  ```
  sudo bash scripts/ci/install-runner.sh <owner/repo> linux
  sudo bash scripts/ci/install-runner.sh <owner/repo> windows
  ```

  Target dirs live in `/srv/ci/desktop/target-<platform>-<profile>` and stay warm between runs.
  Containers get half the cpu shares of a default container and a 9 GB memory cap, so the backend
  builds on the same machine keep priority under load and the desktop takes every core when it is
  idle. Below 25 GB free the other desktop target dirs are dropped before a build.
- macOS builds on `macos-15` (GitHub hosted, Apple silicon). A self-hosted mac is used instead when
  the variable `DESKTOP_MACOS_RUNNER` is set, eg `["self-hosted","macOS","ARM64"]`.

## Secrets and variables

Repository secrets:

| Name | Needed for |
|---|---|
| `RISEONLY_TOOLS_SSH_KEY` | read-only deploy key of riseonly-tools (the canon crates are path dependencies) |
| `APPLE_CERTIFICATE_P12_BASE64`, `APPLE_CERTIFICATE_PASSWORD` | Developer ID Application certificate; without it macOS builds keep an ad hoc signature |
| `APPLE_NOTARY_KEY_P8_BASE64`, `APPLE_NOTARY_KEY_ID`, `APPLE_NOTARY_ISSUER_ID` | notarization (App Store Connect api key) |
| `BUNNY_STORAGE_KEY` | password of the storage zone behind the cdn |
| `BUNNY_API_KEY` | optional, purges the feed from the cdn cache |
| `DESKTOP_FEED_SIGNING_KEY` | ed25519 private key in pem; the publish log prints the public key for the app |

Per environment (`staging`, `production`), optional: `BACKEND_RELEASE_HOOK`, `BACKEND_RELEASE_TOKEN`.
Required reviewers on the `production` environment make every production publish wait for an approval.

Repository variables:

| Name | Default |
|---|---|
| `BUNNY_STORAGE_ZONE` | none, required |
| `BUNNY_STORAGE_HOST` | `storage.bunnycdn.com` |
| `DESKTOP_DOWNLOADS_BASE` | `https://cdn.riseonly.net/desktop` |
| `DESKTOP_STORAGE_PREFIX` | `desktop` |
| `APPLE_SIGNING_IDENTITY` | the first identity in the certificate |
| `DESKTOP_MACOS_RUNNER` | `"macos-15"` |
| `DESKTOP_FEED_PUBLIC_KEY` | none; base64 ed25519 public key built into the app, without it a release build never installs an update |

The riseonly-tools commit ci builds against is `TOOLS_REF` in `.github/ci.env`; it must be pushed.

## Feed key

```
openssl genpkey -algorithm ed25519 -out feed.key
openssl pkey -in feed.key -pubout -outform DER | tail -c 32 | base64
```

The pem goes into the secret `DESKTOP_FEED_SIGNING_KEY`, the base64 public key into the variable
`DESKTOP_FEED_PUBLIC_KEY` (compiled into the app as `RISE_UPDATE_FEED_PUBLIC_KEY`).

## In the app

The app reads `<DESKTOP_DOWNLOADS_BASE>/<environment>/feed.json` 8 s after launch and then every 6 hours
(`RISE_UPDATE_FEED_URL` overrides the url at build time or at run time). With a newer build for its
platform and a valid signature it shows the "Update: N" pill at the bottom of the list column, N being
the number of newer versions. A press downloads the file, checks its size and sha256, swaps it in
(the bundle on macOS, the AppImage on Linux, the folder on Windows with the running exe renamed
aside) and restarts. Installs it cannot replace (a deb, a read only volume) get no pill.
