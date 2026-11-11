# Pipelines for test, build and release

- RFC: 0020
- Status: accepted (built; the pipelines run on the first push)
- Milestone: Release (`todo.md`, "Next milestones")
- Spec: [`spec/readme.md`](../spec/readme.md), open question 4
  (platforms); [`agent.md`](../agent.md), "Before committing"; the
  "Static musl build check" items in [`todo.md`](../todo.md)

## Summary

Every push and merge request is checked by a pipeline on GitHub and on
GitLab: format, `clippy`, tests, and a static release build. A tag
`vX.Y.Z` builds static binaries for Linux (x86_64 and aarch64, musl)
and, on GitHub, macOS, packs them with the readme, the licence and the
shell completions, writes checksums, and publishes a release with notes
from `CHANGELOG.md`. The same tag builds Debian packages (`.deb`, amd64
and arm64) from the static binaries, checked with `lintian` and
installed in a clean Debian and Ubuntu container, and attached to the
release. The crate is made ready for crates.io: metadata, an `include`
list, `cargo package` checked on every push, and a publish step on
release tags, by trusted publishing from GitHub after a first manual
publish. Both pipelines call the same scripts in `ci/`, so what runs is
the same on both forges and on a developer's machine.

## Motivation

- `agent.md` asks for `cargo fmt`, `clippy -D warnings` and `cargo test`
  before each commit. Nothing checks it; a commit from another machine
  or a hurried one goes in unchecked.
- `todo.md` has "Static musl build check" twice, moved to Release because
  the target is not installed locally. Without a pipeline it stays
  unchecked, and a dependency that pulls in a C library (OpenSSL, a
  system `libgit2`) would break the static build unnoticed.
- There is no release: users build from source with a Rust toolchain.
  A coding agent is installed on machines and servers that often have no
  toolchain; one static file is the easiest way in.
- The code lives on GitHub (`origin`), and is also meant to be used on
  GitLab. Writing the logic twice in two YAML dialects would drift.
- On Debian and Ubuntu machines a `.deb` is what admins expect: `apt`
  knows the file, removes it cleanly, and puts the shell completions
  where the shells find them.
- Rust users expect `cargo install clai`. `Cargo.toml` has no
  description, licence or repository, which crates.io requires, and
  nothing checks that the packed crate builds: `themes/` is compiled in
  with `include_str!`, and a forgotten file breaks only the published
  crate.

## Scope

In:

- `ci/` scripts for check, test, build, package and release notes.
- `.github/workflows/ci.yml` and `release.yml`; `.gitlab-ci.yml`.
- Targets, toolchain pinning, caching, artifacts.
- The release process: version, tag, changelog, packages, checksums,
  provenance, release pages on both forges.
- `clai --version` with the commit it was built from.
- Debian packages: contents, metadata, version mapping, `lintian`, an
  install test, attached to the release.
- crates.io: metadata, `include`, the packed crate checked, publishing
  on release tags.
- `readme.md`: installing a release, the `.deb`, `cargo install`, and
  the release process.

Out:

- An apt repository (`apt install clai` after adding a source): the
  `.deb` is a download for now (open question 8).
- Packaging for Debian itself (a source package built from Debian's
  Rust crates, in the Debian archive), RPM, Homebrew, AUR. They can use
  the release archives later.
- A man page (`clap_mangen`); the `.deb` ships the readme instead (open
  question 9).
- Windows: `clai` uses Unix sockets, process groups and signals
  (`nix`); it does not build there.
- Running the live tests (`tests/live.rs`) on every push: they need API
  keys and cost money. A manual job only.
- Code coverage and benchmarks.
- Signing with keys (`minisign`, GPG); provenance attestations instead
  (open question 5).

## Design

### Scripts first

All logic is in POSIX `sh` scripts in `ci/`, which the pipelines only
call. Each runs locally too. **(proposal)**

| Script | Does |
|--------|------|
| `ci/check.sh` | `cargo fmt --all -- --check`; `cargo clippy --all-targets --locked -- -D warnings`. |
| `ci/test.sh` | `cargo test --locked`. |
| `ci/build.sh TARGET` | Release build for `TARGET`; on Linux checks that the binary is static (`file` says `statically linked`, or `ldd` says `not a dynamic executable`); runs `clai --version` and `clai completion bash` when the target runs on the host. |
| `ci/package.sh TARGET VERSION` | `clai-VERSION-TARGET.tar.gz` with `clai`, `readme.md`, `LICENSE`, `CHANGELOG.md`, `completions/` (bash, zsh, fish from `clai completion`). |
| `ci/deb.sh TARGET VERSION` | `clai_DEBVERSION_ARCH.deb` from the built binary with `cargo deb --no-build`; `lintian` on it. |
| `ci/deb-test.sh DEB` | Installs the package with `apt-get install ./DEB` in the running container, checks `clai --version`, the completion files, then `apt-get remove` leaves nothing. |
| `ci/crate.sh` | `cargo package --locked`, then checks the file list against `include` (no `target/`, `rfcs/`, `.github/`) and the size (under 10 MiB). |
| `ci/notes.sh VERSION` | Prints the section of `VERSION` from `CHANGELOG.md`; fails if there is none. |
| `ci/version.sh TAG` | Fails unless `TAG` is `v` plus the `version` in `Cargo.toml`. |

`ci/check.sh` and `ci/test.sh` are what `agent.md` asks for, so
"Before committing" can name them.

### Toolchain

`rust-toolchain.toml` pins the toolchain to one exact version, with
`rustfmt`, `clippy` and the release targets. Both pipelines and local
builds then use the same compiler; a new Rust version comes in by
changing that file, in its own commit, after the pipeline is green.
**(proposal)** `Cargo.toml` gains `rust-version`, the oldest version that
builds (let chains need 1.88). A weekly job builds with the newest
stable to see breakage early.

### Targets

| Target | Built on GitHub | Built on GitLab | Released |
|--------|-----------------|-----------------|----------|
| `x86_64-unknown-linux-musl` | `ubuntu-24.04` | Linux runner | yes |
| `aarch64-unknown-linux-musl` | `ubuntu-24.04-arm` | `cargo zigbuild` on x86_64 | yes |
| `x86_64-apple-darwin` | `macos-15`, cross-built | — | yes, from GitHub |
| `aarch64-apple-darwin` | `macos-15` | — | yes, from GitHub |

- The Linux binaries are static (musl): one file that runs on any
  distribution. `reqwest` uses `rustls` with bundled roots, so there is
  no OpenSSL. Its crypto (`aws-lc-sys`, and `ring` in `Cargo.lock`) is C
  compiled from source into the binary; that needs a C compiler for the
  target: `musl-tools` (`musl-gcc`) for x86_64, `zig` through
  `cargo-zigbuild` when cross-building. The static check in
  `ci/build.sh` keeps a new dependency from linking a system library.
  **(open question 7)**
- GitHub builds aarch64 on its own arm runners, so the binary is also
  run there. GitLab cross-builds with `cargo-zigbuild` (no Docker in
  Docker), and runs the aarch64 binary only if an arm runner is
  registered (`CLAI_ARM_RUNNER` set). **(proposal)**
- macOS: tests also run there on GitHub, since `spec/readme.md` leaves
  the platforms open and `nix` covers it. If macOS is dropped (open
  question 1), these rows go.

### Pipelines

```
push / merge request
  check ──┬── test (linux; macos on GitHub)
          ├── crate (cargo package)
          └── build x86_64 musl (artifact, 7 days)

tag vX.Y.Z
  version ── check ── test ── build (all targets) ──┬── package ──┐
                                                     └── deb ──────┼── release ── crates.io (GitHub only)
                                                         deb-test ─┘

weekly (schedule)
  test on newest stable ── audit (cargo deny)

manual
  live tests (needs the API key variables)
```

| Job | When | Script |
|-----|------|--------|
| `check` | Every push, merge request, tag | `ci/check.sh` |
| `test` | Every push, merge request, tag | `ci/test.sh` |
| `build` | Push: `x86_64-unknown-linux-musl`; tag: all targets | `ci/build.sh` |
| `audit` | Weekly, and when `Cargo.lock` changes | `cargo deny check advisories bans licenses sources` |
| `crate` | Every push, merge request, tag | `ci/crate.sh` |
| `deb` | Tag `v*`; and manual on a branch | `ci/deb.sh` for both Linux targets |
| `deb-test` | After `deb` | `ci/deb-test.sh` in `debian:stable-slim` and `ubuntu:24.04`, on amd64 (arm64 where an arm runner is) |
| `release` | Tag `v*` only | `ci/version.sh`, `ci/package.sh`, `ci/notes.sh`, then the forge's release with the archives and `.deb` files |
| `publish` | Tag `v*` without `-`, after `release`, GitHub only | `cargo publish --locked` |
| `live` | Manual, protected branches only | `cargo test --test live -- --ignored` |

- A new push to a branch cancels the running pipeline of that branch,
  not of `main` or tags.
- Caches: on GitHub `Swatinem/rust-cache`; on GitLab a cache of
  `$CARGO_HOME/registry` and `target/`, keyed by `Cargo.lock` and the
  toolchain file.
- The build of a push keeps its archive for 7 days, so a change can be
  tried without a toolchain.

### GitHub

- `.github/workflows/ci.yml`: push, `pull_request`, schedule, manual.
- `.github/workflows/release.yml`: tags `v*`.
- Actions pinned by commit hash, with the version in a comment;
  Dependabot updates them weekly (`.github/dependabot.yml`, also for
  Cargo).
- `permissions: contents: read` at the top. Only the release job gets
  `contents: write` (the release), `id-token: write` and
  `attestations: write` (provenance).
- Pull requests from forks get no secrets; the `live` job runs only on
  `workflow_dispatch` in the `live` environment, whose secrets hold the
  API keys.
- The release: `gh release create vX.Y.Z` with the archives,
  `SHA256SUMS` and the notes; a version with `-` (`v0.2.0-rc.1`) is a
  pre-release. `actions/attest-build-provenance` attests each archive,
  checkable with `gh attestation verify`.

### GitLab

- `.gitlab-ci.yml` with stages `check`, `test`, `build`, `release`, on
  the image `rust:<pinned version>`, the same as `rust-toolchain.toml`
  (`ci/version.sh` checks they agree). **(proposal)**
- `workflow:rules`: merge requests, the default branch, tags, schedules;
  no duplicate pipeline for a branch with an open merge request.
- The release job runs only for protected tags, uploads the archives
  and `SHA256SUMS` to the project's generic package registry, and
  creates the release with `release-cli`, linking them, with the notes
  from `ci/notes.sh`.
- The live job uses masked, protected variables.
- macOS archives are not built on GitLab (macOS runners are not
  generally available). **(proposal)** The GitLab release links only the
  Linux archives; open question 2 says how the forges relate.

### Debian package

Built with `cargo-deb` from the static musl binary of the same release,
so one package installs on every Debian and Ubuntu release, old or new,
and depends on nothing. **(proposal)**

| Item | Value |
|------|-------|
| File | `clai_0.2.0-1_amd64.deb`, `clai_0.2.0-1_arm64.deb` |
| `Package` | `clai` |
| `Version` | The crate version, `-` before a pre-release part made `~` (`0.2.0~rc.1-1`), so it sorts before `0.2.0-1`; revision `-1`. |
| `Architecture` | `amd64` from `x86_64-unknown-linux-musl`, `arm64` from `aarch64-unknown-linux-musl` |
| `Depends` | none (static); `Recommends: git` (for `clai commit`); `Suggests: wl-clipboard \| xclip \| xsel` (copy, RFC 0016) |
| `Section` / `Priority` | `devel` / `optional` |
| `Maintainer` | From `Cargo.toml` `authors` |
| `Description` | The crate `description`, and the first paragraph of the readme as the long text. |

Files:

| Path | From |
|------|------|
| `/usr/bin/clai` | The release binary. |
| `/usr/share/bash-completion/completions/clai` | `clai completion bash` |
| `/usr/share/zsh/vendor-completions/_clai` | `clai completion zsh` |
| `/usr/share/fish/vendor_completions.d/clai.fish` | `clai completion fish` |
| `/usr/share/doc/clai/README.md` | `readme.md` |
| `/usr/share/doc/clai/changelog.gz` | `CHANGELOG.md`, compressed |
| `/usr/share/doc/clai/copyright` | Machine-readable copyright file, from `LICENSE` and the licences of the dependencies (`cargo deny list`). |

- The completion files are the output of `clai completion SHELL`, saved
  at build time. That code holds no fixed list; it calls
  `COMPLETE=<shell> clai` on each Tab (`spec/command.md`, "clai
  completion"), so the saved files stay right for the installed binary,
  with models and session names, and the user needs no line in
  `~/.bashrc`.
- No maintainer scripts: `clai` has no system service; the server is per
  user and starts on demand. Removing the package leaves the user's
  files in `~/.config/clai` and `~/.local/state/clai`, as Debian expects.
- No `/etc` files: the configuration is per user.
- The metadata lives in `[package.metadata.deb]` in `Cargo.toml`; the
  completions are generated by `ci/deb.sh` into `target/completions/`
  first, which the `assets` list names.
- `lintian` must pass with no errors; warnings that do not apply to a
  static Rust binary (`statically-linked-binary`,
  `no-manual-page`) are overridden in `debian/clai.lintian-overrides`
  with a comment each.
- The `.deb` files go into the release with the archives and are listed
  in `SHA256SUMS`. GitLab also uploads them to its generic package
  registry.

### crates.io

What `cargo install clai` needs **(proposal)**:

- `Cargo.toml` gains `description`, `license`, `repository`,
  `homepage`, `readme = "readme.md"`, `keywords` (at most five: `ai`,
  `agent`, `llm`, `cli`, `tui`), `categories`
  (`command-line-utilities`, `development-tools`), `rust-version`,
  `authors`.
- `include` lists what is packed: `src/**`, `themes/**`, `tests/**`
  (the unit tests read `tests/fixtures` and `tests/configs`, so
  `cargo test` works from the crate), `readme.md`, `CHANGELOG.md`,
  `LICENSE*`. `rfcs/`, `spec/`, `ci/`, `.github/`, `debian/`, `todo.md`
  stay out. `ci/crate.sh` fails when a file in `src/` or `themes/` is
  missing from the packed list or the crate is over 10 MiB.
- All dependencies are from crates.io with versions, as now; `cargo
  package` rejects `path` and `git` dependencies, and `cargo deny`
  (`sources`) keeps it so.
- `src/lib.rs` makes docs.rs build a library page. Its crate docs say
  that the library is the binary's internals and has no stable API;
  semver applies to the command line, the configuration and the
  protocol, not to the Rust items. **(proposal)**
- The name: `clai` must be free on crates.io. If it is taken, the
  package is named otherwise (for example `clai-agent`) and
  `[[bin]] name = "clai"` keeps the command (open question 6).
- Building from crates.io links against glibc and compiles `aws-lc-sys`,
  so the readme says `cargo install clai --locked` needs a C compiler.

Publishing **(proposal)**:

- The first version is published by hand by the owner with
  `cargo publish --locked`, after `ci/crate.sh` passed on the release
  commit. Publishing cannot be undone; a version can only be yanked.
- Then crates.io Trusted Publishing is set up for the GitHub repository
  and `release.yml`: the `publish` job gets a short-lived token through
  OIDC (`id-token: write`, `rust-lang/crates-io-auth-action`), and no
  crates.io token is stored in GitHub or GitLab.
- `publish` runs only for tags without a pre-release part, only on
  GitHub (one forge publishes, so the same version is never published
  twice), and only after the release job succeeded. It runs in the
  `crates-io` environment, which can require the owner's approval.
- A failed `publish` leaves the GitHub release in place; it is retried by
  running the job again.

### Version and changelog

- `Cargo.toml` `version` is the version; the tag is `v` plus it.
  `ci/version.sh` fails the release otherwise.
- `CHANGELOG.md` (new), one section per version, newest first, with an
  `Unreleased` section at the top that changes are written into. The
  release notes are the version's section. **(proposal)**
- `clai --version` prints `clai 0.2.0 (abc1234 2026-11-11)`: the commit
  and date come from `CLAI_BUILD_COMMIT` and `CLAI_BUILD_DATE`, set by
  `ci/build.sh` and read with `option_env!`; a local build without them
  prints `clai 0.2.0`. No `build.rs`, no new dependency.

The release process, for `readme.md`:

1. Move the `Unreleased` entries of `CHANGELOG.md` into a new section
   `0.2.0 - 2026-11-20`.
2. Set `version = "0.2.0"` in `Cargo.toml`; `cargo build` updates
   `Cargo.lock`.
3. Commit, `git tag -a v0.2.0 -m v0.2.0`, push the commit and the tag.
4. The release pipeline builds, packs and publishes; nothing else is
   done by hand.

### Installing

`readme.md` gains:

```sh
curl -LO https://github.com/smolkov/clai/releases/download/v0.2.0/clai-0.2.0-x86_64-unknown-linux-musl.tar.gz
sha256sum -c --ignore-missing SHA256SUMS
tar xf clai-0.2.0-x86_64-unknown-linux-musl.tar.gz
install clai-0.2.0-x86_64-unknown-linux-musl/clai ~/.local/bin/
```

On Debian and Ubuntu:

```sh
sudo apt install ./clai_0.2.0-1_amd64.deb
```

From crates.io (builds from source, needs Rust and a C compiler):

```sh
cargo install clai --locked
```

### Licence

A release ships a licence. `Cargo.toml` has none and there is no
`LICENSE` file. The `release` job fails without one, and `cargo deny`
needs the project's own licence for the `licenses` check. crates.io
refuses a crate without `license`, and the `.deb` needs it for
`copyright`. It is the
user's choice (open question 3); the pipeline is built so that it is
only a file and a key.

## Dependencies

No new crates in `clai`. Tools in the pipelines only, at pinned
versions: `cargo-zigbuild` and `zig` (GitLab aarch64), `cargo-deny`
(audit), `cargo-deb` and `lintian` (Debian packages), `release-cli`
(GitLab), `gh` (GitHub, on the runners), `rust-lang/crates-io-auth-action`
(publishing).

New files: `ci/*.sh`, `.github/workflows/ci.yml`,
`.github/workflows/release.yml`, `.github/dependabot.yml`,
`.gitlab-ci.yml`, `rust-toolchain.toml`, `deny.toml`, `CHANGELOG.md`,
`LICENSE`, `debian/clai.lintian-overrides`. `Cargo.toml` gains the
crates.io metadata, `include` and `[package.metadata.deb]`.

## Spec changes

- `spec/readme.md`: open question 4 (platforms) answered: Linux and
  macOS, not Windows; or as decided.
- `spec/command.md`: the `--version` output with commit and date.
- `agent.md`, "Before committing": `ci/check.sh` and `ci/test.sh` as the
  short form.
- `readme.md`: "Install" with the archives, the `.deb`
  (`sudo apt install ./clai_0.2.0-1_amd64.deb`) and `cargo install clai
  --locked`; "Release" with the process, the first manual crates.io
  publish, and that publishing cannot be undone.
- `todo.md`: the musl check items done; the Release milestone split into
  these steps.

## Testing

- The scripts run locally: `ci/check.sh`, `ci/test.sh`,
  `ci/build.sh x86_64-unknown-linux-musl` on a machine with the target.
- `ci/notes.sh` and `ci/version.sh`: run on a small `CHANGELOG.md` and
  tags with and without a match, in `tests/ci.sh`, called by `ci/test.sh`.
- `ci/deb.sh` and `ci/deb-test.sh` locally in a `debian:stable-slim`
  container: `lintian` clean; install, `clai --version`, `bash -ic
  'complete -p clai'` finds the completion; remove leaves no file of the
  package. A pre-release version maps to `~rc` and sorts before the
  release (`dpkg --compare-versions`).
- `ci/crate.sh`: the packed crate builds and its tests pass
  (`cargo package` builds it; `ci/crate.sh` also runs `cargo test` in the
  unpacked crate); a file left out of `include` fails it.
- `--version`: a test in `tests/cli.rs` that the output starts with
  `clai ` and the version, with and without the build variables.
- The pipelines themselves: a branch with a deliberate `clippy` warning
  fails `check`; a pre-release tag `v0.1.1-rc.1` on a test branch runs
  the whole release on both forges, and the archives are downloaded and
  run on an x86_64 and an aarch64 machine, the `.deb` files installed on
  a Debian and an Ubuntu machine. `publish` is skipped for the
  pre-release. The pre-release is deleted after.
- The first crates.io publish by hand, then the next release by the
  `publish` job; `cargo install clai --locked` on a clean machine.

## Implementation plan

1. **Scripts, toolchain and version**
   - Status: done
   - Files: `ci/*.sh`, `tests/ci.sh`, `rust-toolchain.toml`,
     `Cargo.toml`, `src/cli.rs`, `tests/cli.rs`, `agent.md`
   - Result: check, test and build run locally the same way; the musl
     build is checked static; `--version` shows the commit.
   - Tests: the script tests, `tests/cli.rs`.

2. **GitHub checks**
   - Status: in progress
   - Files: `.github/workflows/ci.yml`, `.github/dependabot.yml`,
     `deny.toml`
   - Result: every push and pull request is checked, tested on Linux
     and macOS, and builds a musl archive; weekly newest stable and
     audit.
   - Tests: a green run; a failing run with a warning.

3. **GitLab checks**
   - Status: in progress
   - Files: `.gitlab-ci.yml`
   - Result: the same checks on GitLab, with the aarch64 cross build.
   - Tests: a green pipeline on the GitLab project.

4. **Release**
   - Status: in progress
   - Files: `.github/workflows/release.yml`, `.gitlab-ci.yml`,
     `CHANGELOG.md`, `LICENSE`, `readme.md`
   - Result: a tag publishes archives, checksums, provenance and notes
     on both forges.
   - Tests: the pre-release run above.

5. **Debian package**
   - Status: done
   - Files: `ci/deb.sh`, `ci/deb-test.sh`, `Cargo.toml`
     (`[package.metadata.deb]`), `debian/clai.lintian-overrides`, the
     workflows, `.gitlab-ci.yml`
   - Result: a tag adds `.deb` files for amd64 and arm64 to the release,
     `lintian` clean and install-tested.
   - Tests: the Debian tests above; the pre-release run.

6. **Ready for crates.io**
   - Status: done
   - Files: `Cargo.toml` (metadata, `include`), `src/lib.rs` (crate
     docs), `ci/crate.sh`, the workflows, `.gitlab-ci.yml`, `deny.toml`
   - Result: every push checks the packed crate; the name is decided.
   - Tests: `ci/crate.sh`.

7. **Publish to crates.io**
   - Status: in progress
   - Files: `.github/workflows/release.yml`, `readme.md`
   - Result: the first version published by hand; trusted publishing
     set up; later release tags publish from GitHub.
   - Tests: the first publish, then `cargo install clai --locked`.

8. **Docs and spec**
   - Status: done
   - Files: `readme.md`, `spec/readme.md`, `spec/command.md`, `todo.md`
   - Result: install and release are documented; the platform question
     is answered.
   - Tests: `ci/check.sh` and `ci/test.sh` clean.

## Alternatives

- **Logic in the YAML of each forge.** Shorter to start, but two copies
  that drift, and nothing to run locally when a job fails.
- **`cargo xtask` in Rust instead of `sh`.** Portable and typed, but a
  workspace and a second crate for a few commands. The scripts are
  short; `sh` is on every runner.
- **`cross` for the aarch64 build.** Needs Docker, which GitLab shared
  runners give only through Docker in Docker. `cargo-zigbuild` works in
  the plain Rust image, and GitHub has native arm runners.
- **glibc builds instead of musl.** Faster allocator, but the binary
  depends on the glibc version of the build machine. If the musl
  allocator shows up in profiles, `mimalloc` can be added later.
- **`cargo-dist` or `release-plz`.** Generate the release pipeline,
  installers and changelogs. They are GitHub-first, write their own
  YAML, and would make the GitLab side the odd one out.
- **`dpkg-deb` with a hand-written `debian/` tree, or `nfpm`.** Full
  control, or one tool for `.deb` and `.rpm`, but the metadata is kept
  apart from `Cargo.toml`. `cargo-deb` reads name, version, description
  and licence from it, and can pack a prebuilt binary.
- **A `.deb` per distribution built against its glibc.** What Debian
  itself does; many packages to build and test for no gain over one
  static binary.
- **Publishing to crates.io with a stored token.** Simpler to set up,
  but a long-lived secret that can publish any version; trusted
  publishing gives a token only to the release workflow, for minutes.
- **Release from one forge and mirror to the other.** Simpler (see open
  question 2), but GitLab users would download from GitHub.

## Open questions

1. Platforms: Linux and macOS (proposal), or Linux only? This also
   answers `spec/readme.md`, open question 4.
2. How do the forges relate: GitHub the source with GitLab as a pull
   mirror that also releases (proposal), GitLab as the source, or a
   release on GitHub only?
3. Which licence: MIT, Apache-2.0, both (as most Rust crates), or
   another?
4. Pin the toolchain in `rust-toolchain.toml` (proposal), or follow
   stable?
5. Is provenance (GitHub attestations) enough, or should archives also
   be signed with a key (`minisign`, cosign)?
6. The crates.io name: `clai` if it is free (check with `cargo info
   clai` before step 6), otherwise which (`clai-agent`, `clai-cli`)? The
   command stays `clai` either way.
7. `rustls` brings `aws-lc-sys`, which needs a C (and on some targets a
   CMake) toolchain for each target. Keep it, or switch `reqwest` to the
   `ring` provider, which cross-builds more easily?

8. An apt repository later (`apt install clai` after adding a source:
   GitLab's Debian registry, a self-hosted repository with `reprepro`,
   or a hosted service), or the `.deb` as a download only (proposal)?
9. A man page (`clap_mangen`, `clai.1` in the `.deb` and the archives),
   or the readme only (proposal)?
10. Publish pre-releases (`0.2.0-rc.1`) to crates.io too, or releases
    only (proposal)?

## Answers

Given by the user before building:

1. Platforms: Linux and macOS.
2. Forges: GitHub is the source; GitLab is a mirror that runs the same
   checks and releases the Linux files.
3. Licence: `MIT OR Apache-2.0` (`LICENSE-MIT`, `LICENSE-APACHE`).
4. The toolchain is pinned (`rust-toolchain.toml`, 1.99.1).
5. Provenance attestations, no signing key.
6. The crate name `clai` was free on crates.io (checked 2026-11-11 with
   `cargo info clai`); it is not reserved until the first publish.
7. `aws-lc-sys` stays; the builds use `musl-gcc` or zig.
8. The `.deb` is a download; no apt repository.
9. No man page; lintian's `no-manual-page` is overridden.
10. Pre-releases are not published to crates.io.

`authors` and the Debian `Maintainer` carry the user's name and personal
address, as they chose.

## Notes from building

- Steps 2, 3, 4 and 7 are written but can only be finished on the forges:
  the first push runs `ci.yml` and `.gitlab-ci.yml`, a pre-release tag
  runs the release, and the first crates.io publish is by hand. Locally
  checked: `ci/check.sh`, `ci/test.sh` (765 unit tests and the
  integration tests, 17 script tests), `ci/crate.sh` (the packed crate
  builds and its tests pass; 473 KB, 274 files), `cargo deny` clean, the
  static x86_64 musl build in the `rust:1.99.1` image (`static-pie
  linked`), `ci/package.sh`, `ci/deb.sh` with `lintian` clean in
  `debian:stable-slim`, and `ci/deb-test.sh` in `debian:stable-slim` and
  `ubuntu:24.04`. Not checked locally: aarch64 (zig and arm runners), macOS,
  `rust-version = "1.88"` (the weekly `msrv` job checks it; 1.88 is the
  highest `rust-version` among the dependencies built for these targets).
- GitHub retired `macos-13`; both macOS targets are built on `macos-15`
  (arm64), x86_64 by cross-compiling with the Apple SDK. That binary is not
  started in the pipeline.
- `clai completion` writes the path it was started by into the script.
  `ci/completions.sh` runs the binary as `clai` through `PATH`, so the
  saved files call `clai`; it fails if a build path is left in them. No
  code change was needed.
- Helpers beyond the RFC's table: `ci/common.sh` (shared functions),
  `ci/completions.sh`, `ci/host-bin.sh` (a binary that runs on the host,
  for cross targets; `CLAI_HOST_BIN` overrides), `ci/checksums.sh`.
  `ci/version.sh` gained `deb` and `prerelease`. `ci/build.sh` sets
  `CC_<target>=musl-gcc` for native musl builds.
- Debian: lintian wants a Debian changelog, so `ci/deb.sh` writes a short
  `changelog.Debian.gz` pointing to the project's `changelog.gz`. The
  overrides (`debian/clai.lintian-overrides`, installed into the package)
  are `empty-field Depends`, `no-manual-page`,
  `shared-library-lacks-prerequisites` (lintian takes a static-pie binary
  for a library) and `initial-upload-closes-no-bugs`. Slim images do not
  install `/usr/share/doc`, so `ci/deb-test.sh` checks the documents in the
  package listing. The Debian synopsis is the crate `description`, so it
  was shortened to fit 80 characters and not start with an article.
- `deny.toml` ignores RUSTSEC-2025-0141 (bincode, unmaintained, not
  vulnerable; used by syntect), with the reason.
- The GitLab release uses the `release:` keyword with
  `release-cli:v0.24.0`, and links the files in the generic package
  registry; versions and notes pass between jobs as a dotenv artifact.
- Actions are pinned to the commits of checkout v7.0.1, upload-artifact
  v7.0.2, download-artifact v8.0.2, rust-cache v2.9.3,
  attest-build-provenance v4.2.2 and crates-io-auth-action v1.0.6.
- `dist/` is in `.gitignore`.

