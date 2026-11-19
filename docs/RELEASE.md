# Build and release

Branches: `development` -> `staging` -> `main`. Everything goes through pull requests.

## Labels

| Label | Who sets it | What it does |
|---|---|---|
| `build` | ci, on every pull request into `staging` or `main` | builds the picked platforms and uploads them as workflow artifacts (7 days) |
| `deploy` | you | after the merge the build is released; without it a merge ships nothing |
| `linux` `win` `macos` | you | pick a platform with every architecture of it: Linux and Windows x86_64 and arm64, both Macs |
| `linux-x64` `linux-arm64` | you | pick the Linux architectures (AppImage) |
| `win-x64` `win-arm64` | you | pick the Windows architectures (zip) |
| `macos-silicon` `macos-intel` | you | pick the Mac architectures |
| `env-dev` `env-staging` `env-prod` | you | pull request builds only: the backend the app talks to, instead of the branch default (staging, production into `main`); one at a time |
| `release-profile` | you | pull request builds only: the `release` profile (lto) instead of `fast` |
| `tests` | you | also runs `cargo test` and clippy on Linux |

With no platform or architecture label at all, Linux x86_64, Windows x86_64 and both Macs build.
Labels mix: `linux-arm64` and `macos-intel` build exactly those two. A release (`deploy`) builds
the same picked architectures, always against its branch environment and with the release profile.

Any other pull request builds only when you add `build` yourself. Removing `build` stops builds on
later pushes. The labels are created by ci on the first pull request.

## Flows

- **pull request** (`build.yml`): lint (fmt, boundaries, module tree, canon locales) and a `fast`
  profile build of each picked platform. Into `main` it is a production app, otherwise a staging app.
- **development -> staging, merged with `deploy`** (`release.yml`): release profile build, packaged
  as the staging app (`Riseonly Staging`, talks to the staging backend), uploaded to the cdn under
  `staging/`, the staging feed updated, a draft GitHub release. The compiled binaries stay as
  `payload-<key>-v<version>-b<build>-<platform>-<arch>` artifacts for 90 days.
- **staging -> main, merged with `deploy`**: the same for production: the production feed, a
  published GitHub release with tag `v<version>`. It fails early when that tag exists. When the staging release of the
  same sources left its binaries, nothing compiles: they are packaged again as the production app
  under the staging build number, signed and notarized. A platform with no kept binary compiles.

Any other merge into `staging` or `main` (another head branch, no `deploy`) releases nothing.

Staging releases are switched off: a merge into `staging` publishes nothing, a pull request build
is a production app, and no staging build, draft or link shows up in the repository. Production
then compiles on the merge into `main`. The repository variable `DESKTOP_STAGING_RELEASES=on`
brings the staging step back; it publishes to the cdn only, never as a GitHub release.

A production version can gain platforms later: a release whose platforms are not yet in that
version adds its files to the same GitHub release and feed entry. A platform that is already there
is refused, the plan names it.

## Versions

Nobody edits the version by hand. A release into staging whose sources have no kept binary gets the
next version: the highest one ever published to staging or production, moved by the label of the
merged pull request. `release-patch` (1.1.4 -> 1.1.5) is put on every pull request into staging by
ci, `release-minor` (1.2.0) and `release-major` (2.0.0) are put on by hand and take `release-patch`
off. Minor and patch are single digits, as in the iOS app: 1.0.9 is followed by 1.1.0, 1.9.9 by
2.0.0. `set-version.py` writes that version into `Cargo.toml` and `Cargo.lock` of the job before it
compiles or packages, nothing is committed; the version in the repository is only a floor, raise it
to jump further than a label does. A release that finds a kept binary keeps the version of that
binary: publishing the same sources again, and the release into production, which ships what staging
shipped under the same number. A pull request build carries the version of the repository. The
updater compares versions, so a staging install moves on only when the sources changed.

## One build, two apps

The binary does not carry its environment, the package names it. The file `environment` beside
`assets` holds the canon name (`Contents/Resources/environment` on macOS, `share/riseonly/environment`
on Linux, next to `riseonly.exe` on Windows) and the app reads it at start. Without the file the
build value `RISE_ENV` decides, which is what `cargo run` and the watch loop rely on. ci compiles
with no `RISE_ENV` and packages for the environment of the pull request or release, so staging and
production ship one binary. On macOS each package is signed and notarized on its own, the name and
the bundle id differ. On Windows the product name in the file properties is `Riseonly` in both. On
Linux and Windows the file is plain and editable; the backend, not the client, decides who gets in.

A kept binary is found by the key of the sources that go into it: every file but `.github/workflows`,
`docs`, `scripts/bundle-*` and everything in `scripts/ci` but the `build-*.sh` scripts, the Linux
and Windows toolchain image included (`python3 scripts/ci/plan.py key` prints it), so a
change to those compiles nothing again. A binary compiled on another machine joins a release the
same way: `scripts/ci/payload.sh <build> macos-aarch64=<riseonly> macos-x86_64=<riseonly>` stores
them in the prerelease `payload-<key>-v<version>-b<build>`, and the macOS jobs of the next release of these
sources package, sign and notarize them instead of compiling. Compile such a binary the way
`build-macos.sh` does: no `RISE_ENV`, the `RISE_BUILD_NUMBER` of that build, the
`RISE_UPDATE_FEED_PUBLIC_KEY` of the repository.

The label `macos-local` moves the macOS jobs of a pull request or a release from GitHub machines to
the runner `riseonly-desktop-macos` (a Mac of the team, `~/actions-runner/riseonly-desktop`, a
launchd agent of the logged in user). The job is the same: it compiles when no kept binary fits,
packages, signs and notarizes with the repository secrets. There the checkout keeps `target` and
`vendor/ffmpeg` between runs, the Rust toolchain and the cargo home are the machine's own, and the
build waits for `/tmp/riseonly-desktop-build.lock`. The label alone picks both Mac builds; a pull
request from a fork never reaches that runner. The Mac has to be on and logged in, else the job
waits in the queue.

## Where the files go

```
<DESKTOP_DOWNLOADS_BASE>/<environment>/<version>-<build>/<Slug>-<version>-macos-aarch64.zip   updater
<DESKTOP_DOWNLOADS_BASE>/<environment>/<version>-<build>/<Slug>-<version>-macos-aarch64.dmg   first install
<DESKTOP_DOWNLOADS_BASE>/<environment>/<version>-<build>/<Slug>-<version>-macos-x86_64.zip    the same for Intel Macs
<DESKTOP_DOWNLOADS_BASE>/<environment>/<version>-<build>/<Slug>-<version>-macos-x86_64.dmg
<DESKTOP_DOWNLOADS_BASE>/<environment>/<version>-<build>/<Slug>-<version>-linux-x86_64.AppImage
<DESKTOP_DOWNLOADS_BASE>/<environment>/<version>-<build>/<Slug>-<version>-windows-x86_64.zip
<DESKTOP_DOWNLOADS_BASE>/<environment>/feed.json       the feed rise_platform::updater::UpdateManifest reads
<DESKTOP_DOWNLOADS_BASE>/<environment>/feed.json.sig   base64 ed25519 signature of feed.json
<DESKTOP_DOWNLOADS_BASE>/<environment>/latest/<Slug>-macos-aarch64.dmg   always the newest build of that file
<DESKTOP_DOWNLOADS_BASE>/<environment>/latest/<Slug>-macos-x86_64.dmg
<DESKTOP_DOWNLOADS_BASE>/<environment>/releases.json   every published version, never trimmed
<DESKTOP_DOWNLOADS_BASE>/<environment>/index.html      public page with the latest links and all versions
```

Each Mac build reaches only installs of its own architecture: the app compiles its feed key
(`macos-aarch64` or `macos-x86_64`) and never takes the other one, and a release without a build for
its architecture offers it no update. A release that ships only `macos-silicon` leaves Intel installs
on the version they have.

`feed.json` keeps the last 12 versions. Each release carries `artifacts` (what the updater installs,
per target) and `downloads` (every file, the dmg included, for the site). A release with fewer
platforms keeps the other targets of the same version.

`latest/` holds a copy of every file of the newest version under a name without the version
(`Riseonly-macos-aarch64.dmg`, `Riseonly-Staging-macos-aarch64.dmg`), a fixed url to hand out.
Publishing an older version again leaves `latest/` alone. The versions page and `releases.json`
link the exact files (`url`), the fixed one is `stable_url`.

A cached file is never replaced under its url: the cdn keeps a big file in 5 MB pieces, and after
a replace an edge that served the old file answers with old and new pieces mixed, the right size and
the wrong bytes; neither a url purge nor a folder purge clears it. So publishing a version and build
that already has a folder goes to a new one, `<version>-<build>.<run>-<attempt>`, and `latest/`,
the one place replaced in place, is not cached on the cdn at all.

The pull zone caches for 30 days by default, so the files that change in place have two edge rules
with cdn and browser cache time 0: `feed.json`, `releases.json` and `index.html` under `desktop/`,
and `desktop/*/latest/*`. The publish step purges what it replaced with `BUNNY_API_KEY`, then
downloads every file it uploaded through the cdn and compares it with the build; that check sees
one edge only.

The zone has no allowed referrers list. A third edge rule keeps the hotlink protection for the app
media: a request outside `desktop/` whose `Referer` starts with `http` and is not riseonly.net or
a subdomain is blocked. Everything under `desktop/` answers any referer, so a download link works
from GitHub, a chat or a search page.

The publish step also posts the release as json to `BACKEND_RELEASE_HOOK` when that secret is set.
A failed hook is a warning, the feed is already out.

## Machines

- Linux x86_64 and both Windows architectures build on the ci builder, in docker
  (`scripts/ci/toolchain/Dockerfile`, Ubuntu 24.04, so the Linux build needs glibc 2.39 and libstdc++ of GCC 13 (Ubuntu 24.04, Debian 13, Fedora 40 and newer); 22.04 has PipeWire headers too old for libspa 0.8 and a GCC that rejects the WebRTC headers; the AppImage carries libpipewire and libuv (MapLibre), libcurl comes from the system, llvm 19, cargo-xwin for
  msvc, mingw-w64 and llvm-mingw for the Windows FFmpeg dlls). Windows arm64 is cross compiled
  there too (`WIN_ARCH=aarch64`); the two Windows jobs run one after another. Linux arm64 builds
  on the GitHub hosted `ubuntu-24.04-arm` runner in the same image (override with the
  `DESKTOP_LINUX_ARM_RUNNER` variable), with a 14 GB container cap. A release build there runs 3 rustc at a time (`DESKTOP_CI_JOBS`) and splits each crate into 16 codegen units (`CARGO_PROFILE_RELEASE_CODEGEN_UNITS`): with one unit `riseonly-data-mid` alone outgrew the 14 GB container, which has no swap. Thin LTO still optimizes across them; the Mac builds keep one unit. One runner per platform so
  both build at once:

  ```
  sudo bash scripts/ci/install-runner.sh <owner/repo> linux
  sudo bash scripts/ci/install-runner.sh <owner/repo> windows
  ```

  Target dirs live in `/srv/ci/desktop/target-<platform>-<profile>` and stay warm between runs.
  Containers get half the cpu shares of a default container and a 9 GB memory cap, so the backend
  builds on the same machine keep priority under load and the desktop takes every core when it is
  idle. Below 25 GB free the other desktop target dirs are dropped before a build.
- macOS builds on `macos-15` (GitHub hosted, Apple silicon), one job per architecture: `aarch64`
  natively and `x86_64` for Intel Macs cross compiled on the same runner (`MACOS_ARCH=x86_64`: rust
  target `x86_64-apple-darwin`, FFmpeg built under Rosetta with nasm and found by pkg-config only in
  `vendor/ffmpeg/macos-x86_64`, the MapLibre core built from source with ninja because
  maplibre-native publishes no prebuilt Intel macOS core, so that job takes longer).
  A self-hosted mac is used instead when the variable `DESKTOP_MACOS_RUNNER` is set, eg
  `["self-hosted","macOS","ARM64"]`.

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

## Known gaps

- 1.1.5 shipped for macOS and Linux first; Windows x64 shipped as 1.1.6 from the same sources (the FFmpeg tarball fix changed a build script), Windows arm64 joins 1.1.6 once the MapLibre core builds.
- Windows arm64: the static MapLibre core does not build for `aarch64-pc-windows-msvc` yet (the
  core build stops inside the libpng step). Release `win-x64` until it does; Windows on Arm runs the
  x64 build under emulation.
- Windows FFmpeg cache: a prefix restored from `$target/ffmpeg-windows-*` comes without
  `vendor/ffmpeg/ffmpeg-<version>.tar.xz`, which the LGPL bundle must carry, so packaging stops
  after a full compile. `scripts/ci/build-windows.sh` has to keep the tarball next to the cached
  prefix and fetch it again (pinned sha256) when it is missing.
