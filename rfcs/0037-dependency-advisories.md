# Dependency advisories

- RFC: 0037
- Status: done
- Milestone: none; it closes gaps that a review of the supply chain found
- Spec: [`spec/security.md`](../spec/security.md); RFC 0020 (pipelines),
  `deny.toml`, `.github/workflows/`, `.gitlab-ci.yml`, `AGENTS.md`

## Summary

`codeagent` already checks its dependencies against the RustSec advisory
database with `cargo deny` (RFC 0020). The checks have gaps in timing. The
release pipelines publish without them. A pushed `Cargo.lock` on GitHub
waits for the next Monday. A new advisory against a released version
reaches nobody. And `spec/security.md` does not name dependencies at all.
This RFC:

1. Adds the supply chain to `spec/security.md`: a dependency as an
   attacker, its entry points, its guards, the accepted risks. It also adds
   how a new advisory is handled.
2. Adds one script, `ci/audit.sh`, used by both pipelines and before a
   commit that changes `Cargo.lock`.
3. Makes the audit a gate of every release, on GitHub and GitLab.
4. Runs the audit daily, and on every push that changes `Cargo.lock` or
   `deny.toml`.
5. Builds release binaries with `cargo auditable`, so that an installed
   `codeagent` can be checked against advisories published after its
   release.
6. Gives each ignored advisory a review date that the audit enforces.

## Motivation

A check of the current state, on 2026-11-17:

```
$ cargo deny --locked check advisories bans licenses sources
advisories ok, bans ok, licenses ok, sources ok
```

There are three duplicate-version warnings (`bitflags`, `core-foundation`,
`syn`), and one advisory is ignored with a reason: `RUSTSEC-2025-0141`,
bincode is unmaintained, via syntect. So today nothing is known against the
locked dependencies. The findings are about what happens *tomorrow*, when
an advisory is published.

### Findings

| # | Severity | Finding | Place |
|---|----------|---------|-------|
| 1 | medium | A release does not wait for the audit. On GitHub, `release.yml` has no audit job; `build` needs `check` and `test` only. On GitLab the release jobs use `needs:` without `audit`, so they do not wait for it, and a failed audit does not stop them. A tag can publish binaries, Debian packages and the crate with a dependency that has a known advisory. | `.github/workflows/release.yml`, `.gitlab-ci.yml` (`release`, `upload`) |
| 2 | medium | On GitHub the audit runs on pull requests and on Mondays only. A push to `main` that changes `Cargo.lock` is not audited until the next Monday. Pushes straight to `main` are this repository's usual way. An advisory published on a Tuesday is seen up to six days later. | `.github/workflows/ci.yml`, `audit` |
| 3 | low | No local command. `AGENTS.md`, "Before committing", runs fmt, clippy and tests, not the audit. A dependency update is committed without it. | `AGENTS.md`, `ci/` |
| 4 | low | Released binaries say nothing about what is in them. After a release, a new advisory against one of its crates cannot be matched to an installed `codeagent` by the user or by a scanner (`cargo audit bin`, Trivy, Grype). The user has no way to learn that their version is affected. | `ci/build.sh` |
| 5 | low | Ignored advisories have no expiry. `RUSTSEC-2025-0141` is ignored with a reason but no date. An ignore that is never looked at again can outlive its reason, for example when the crate later gets a real vulnerability under a new advisory. That one is not ignored, but nothing prompts a review of the old reason. | `deny.toml` |
| 6 | open | `spec/security.md` names no dependency as an attacker. It does not say that crates run code at build time (`build.rs`, procedural macros), or what is done when an advisory appears. | `spec/security.md` |

Checked and sound:

| Area | Why it holds | Place |
|------|--------------|-------|
| Advisory policy | Every RustSec advisory fails, also unmaintained and yanked crates; an exception needs an entry with a reason. | `deny.toml`, `[advisories]` |
| Sources | Only crates.io; unknown registries and git sources are denied. | `deny.toml`, `[sources]` |
| Locked builds | Every build, test and install uses `--locked`. | `ci/*.sh`, workflows |
| CI actions | Pinned by commit, version in a comment, updated by Dependabot. | `.github/workflows/` |
| CI tools | `cargo-deny`, `cargo-zigbuild`, `cargo-deb` installed `--locked` at fixed versions; Zig checked by SHA-256. | workflows, `.gitlab-ci.yml` |
| Toolchain | Pinned in `rust-toolchain.toml`; `ci/version.sh toolchain` checks the pipelines use the same. | `rust-toolchain.toml` |
| Updates | Dependabot opens weekly PRs for crates and actions; each PR runs the audit. | `.github/dependabot.yml` |
| Provenance | Release files have build attestations (`gh attestation verify`). | `release.yml` |
| No `unsafe` | `unsafe_code = "forbid"` in `codeagent` itself. It narrows what a memory bug in our code can do; it says nothing about dependencies. | `Cargo.toml` |

## Scope

In:

- The supply-chain part of `spec/security.md`.
- `ci/audit.sh`, and its use in both pipelines and in `AGENTS.md`.
- The audit as a gate of releases; daily and on-change runs.
- `cargo auditable` for release binaries.
- Review dates for ignored advisories.

Out:

- Vetting the code of dependencies (`cargo vet`, `cargo crev`). That is
  real work per update; see open question 3.
- Reproducible builds.
- An in-app warning that the running version is affected; see open
  question 2.
- Dependencies of the scripts' own tools beyond their pinned versions.

## Design

### The supply chain in the spec

`spec/security.md` gains:

- **Asset:** the integrity of what users install, meaning binaries,
  packages and the crate.
- **Attacker:** a dependency. That means a crate with a vulnerability, or
  a crate that a malicious update or a taken-over account has changed,
  including the build tools of the pipelines.
- **Entry points and guards**, one row each:

  | Source | Trusted | Guard |
  |--------|---------|-------|
  | Crates (code at run time and at build time: `build.rs`, procedural macros) | Partly: reviewed when added (an RFC for each new crate), not per update | `cargo deny` against RustSec on every change of `Cargo.lock`, daily, and before every release; crates.io only; `--locked` everywhere |
  | Toolchain and CI tools | Partly | Pinned versions, `--locked`, checksums, actions pinned by commit |
  | Released files | — | Attestations; the dependency list built into the binary (`cargo auditable`) |

- **Accepted risks:** a crate update is not read line by line; build-time
  code runs on the build machine and in CI with their rights; an advisory
  is only as fast as RustSec.
- **Handling an advisory** (a new section):
  1. The daily audit fails and names the advisory.
  2. If it affects `codeagent`, update the crate (`cargo update -p NAME`)
     and release a patch version. Affected means the vulnerable code is
     reachable, or the use is unclear. For `high` or `critical`, do it
     within 7 days **(proposal)**. The CHANGELOG entry names the advisory.
  3. If no fix exists or the code is not reachable, add an `ignore` entry
     with the reason and a review date at most 90 days ahead
     **(proposal)**. Record it in the findings table.
  4. An unmaintained crate gets an issue in `todo.md` to replace it.

### One script

`ci/audit.sh` runs `cargo deny --locked check advisories bans licenses
sources`, plus the review-date check below. Without `cargo-deny` it fails
with `cargo-deny is not installed: cargo install --locked
cargo-deny@<version>`. The version comes from one place,
`ci/common.sh`, which `ci/version.sh` already keeps in step with the
pipelines. Both pipelines call the script instead of their inline
commands, as for the other checks.

`AGENTS.md`, "Before committing", gains one line: *when `Cargo.lock` or
`deny.toml` changes, also `ci/audit.sh`*.

### When the audit runs

| Pipeline | Now | After |
|----------|-----|-------|
| GitHub `ci.yml` | pull requests, Monday, manual | pull requests, **push to `main` that changes `Cargo.lock` or `deny.toml`**, **daily** (its own cron entry; the other Monday jobs stay weekly), manual |
| GitHub `release.yml` | never | **a job `audit`; `build` needs it** |
| GitLab | schedule, web, changes of `Cargo.lock`/`deny.toml` | the same, and **`audit` in the `needs:` of `upload` and `release`**; the README of the pipeline says to set the schedule to daily |

The release gate runs on the tag's own `Cargo.lock`. A failing audit
stops the release before anything is built. A known advisory that is
accepted must therefore be in `deny.toml` before the tag, with its reason.

### Binaries that can be audited

`ci/build.sh` builds release binaries with `cargo auditable build` instead
of `cargo build` (and `cargo auditable zigbuild` for cross builds, which
`cargo-auditable` supports through `CARGO`) **(proposal)**. It adds a
compressed list of the crates and their versions to a section of the
binary. It is a few kilobytes, and nothing changes at run time. Then:

```sh
cargo audit bin ~/.local/bin/codeagent     # or: trivy fs, grype
```

checks an installed `codeagent` against today's advisories. The readme's
"Security" section says so. `cargo-auditable` is a build tool, installed
`--locked` at a fixed version like `cargo-deb`. It is not a dependency of
the crate. The `.deb` gets the same binary.

### Review dates for ignores

Each `ignore` entry's reason ends with `review YYYY-MM-DD`:

```toml
ignore = [
  { id = "RUSTSEC-2025-0141", reason = "unmaintained, no known vulnerability; via syntect; review 2027-02-15" },
]
```

`ci/audit.sh` fails, with the advisory and its date, when an entry has no
review date or the date has passed. A review either moves the date
(the reason still holds) or removes the entry. `cargo deny` itself already
warns when an ignored advisory no longer matches any crate.

## Spec changes

- `spec/security.md`: the asset, the attacker, three rows of entry
  points, the accepted risks, the new section "Dependency advisories", and
  this RFC's findings.
- `readme.md`: "Security" names `cargo audit bin` for an installed
  binary; "Release" names the audit gate.
- `AGENTS.md`: the line under "Before committing".
- `rfcs/0020-ci-pipelines.md`: a note that RFC 0037 changed the audit
  jobs.
- `todo.md`, `CHANGELOG.md`.

## Testing

- `ci/test.sh` already tests the `ci/` scripts. It gains tests of
  `ci/audit.sh`:
  - A `deny.toml` fixture with an ignore whose review date has passed
    fails, and the message names the advisory.
  - One without a date fails.
  - One with a future date passes the date check.
  - Without `cargo-deny` on `PATH`, the message names the install
    command.
- The workflows: `ci/version.sh` checks that the pinned `cargo-deny` and
  `cargo-auditable` versions are the same in both pipelines and in
  `ci/common.sh`.
- By hand, once: a tag on a fork with an advisory left unignored stops
  before `build`. A release binary passes `cargo audit bin`, and
  `objdump -h` shows the `.dep-v0` section.

## Implementation plan

1. **Spec**
   - Status: done
   - Files: `spec/security.md`, `AGENTS.md`
   - Result: the supply chain and advisory handling are written down.
   - Tests: the spec book builds.

2. **`ci/audit.sh` and review dates**
   - Status: done
   - Files: `ci/audit.sh`, `ci/common.sh`, `ci/test.sh`, `deny.toml`
   - Result: one local and CI command; ignores carry a review date.
   - Tests: the script tests above.

3. **Pipelines**
   - Status: done
   - Files: `.github/workflows/ci.yml`, `.github/workflows/release.yml`,
     `.gitlab-ci.yml`, `ci/version.sh`
   - Result: audit daily, on lock changes, and as a release gate.
   - Tests: `ci/version.sh`; the manual tag check.

4. **Auditable binaries**
   - Status: done
   - Files: `ci/build.sh`, workflows (install), `readme.md`
   - Result: release binaries carry their dependency list.
   - Tests: `cargo audit bin` on a built binary.

5. **Docs**
   - Status: done
   - Files: `readme.md`, `rfcs/0020-ci-pipelines.md`, `todo.md`,
     `CHANGELOG.md`, `rfcs/readme.md`
   - Result: users know how to check their binary; maintainers know what
     to do with an advisory.

## Alternatives

- **`cargo audit` instead of `cargo deny`.** It uses the same database, but
  `cargo deny` is already set up and also checks licences, sources and
  bans. Rejected; `cargo audit` is only named for users who check a
  binary.
- **GitHub's Dependabot alerts alone.** They depend on a repository
  setting, do not exist on GitLab, and do not gate a release. They stay
  welcome as an extra.
- **Fail the audit on `high` advisories only.** A severity is often
  missing in RustSec, and "unmaintained" has none. Failing on all, with
  reviewed ignores, is clearer. Rejected.
- **An SBOM file (CycloneDX) with each release** in place of
  `cargo auditable`. A separate file gets lost; the section stays with the
  binary. A later addition, if users ask for it.

## Open questions

1. The time limits for handling an advisory: 7 days for `high` and
   `critical`, 90 days at most for an ignore. Are those right for a
   one-maintainer project?
2. Should `codeagent` itself warn when it runs a version with a known
   advisory (a check against RustSec or a list on the releases page)?
   Proposal: no; it would mean network calls the user did not ask for.
   `cargo audit bin` covers it.
3. Should dependency updates be vetted (`cargo vet` with imported audits
   from Mozilla and Google), at least for crates with build-time code?
   Proposal: not now; reconsider when a second maintainer joins.
4. The duplicate versions (`bitflags`, `core-foundation`, `syn`) are
   warnings. Should they be cleaned up in this RFC? Proposal: no; they come
   from upstream and disappear with updates.

All answered as proposed when the RFC was built: 7 days for `high` and
`critical`, 90 days at most for an ignore (1); no warning in `codeagent`
itself (2); no `cargo vet` for now (3); the duplicate versions stay (4).

## Changes while building

- **GitHub audits every push to `main`**, not only those that change
  `Cargo.lock` or `deny.toml`. A job cannot filter a push by its paths
  without a workflow-wide filter or another action; installing
  `cargo-deny` takes a minute, and auditing every push also catches an
  advisory between two daily runs. The daily run is a second cron entry;
  `check`, `test`, `crate` and `build` skip it, and `stable` and `msrv`
  keep to the Monday entry.
- **GitLab audits every tag** (`if: $CI_COMMIT_TAG`), not only by
  `changes:`, and `upload` needs `audit`, so `release` waits for it
  through `upload`.
- **`ci/build.sh` uses `cargo auditable` when it is installed**, and
  `CODEAGENT_REQUIRE_AUDITABLE=1` (set by both release pipelines) makes it
  required, like `CODEAGENT_REQUIRE_LINTIAN` for the `.deb`. On Linux it
  checks with `readelf` that the binary has its `.dep-v0` section. GitLab
  installs `cargo-auditable` only for tags.
- **The pinned versions** of `cargo-deny` (0.20.2) and `cargo-auditable`
  (0.7.7) are in `ci/common.sh`; `ci/version.sh tools`, run by the
  `check` jobs, compares them with `.gitlab-ci.yml` and every
  `TOOL@VERSION` in the GitHub workflows.
- **The 90-day limit for a review date is a rule, not a check**: `date`
  arithmetic differs between GNU and BSD, and the scripts are POSIX sh.
  `ci/audit.sh` checks that each ignore has a date and that it has not
  passed. `CODEAGENT_TODAY` replaces today's date for the tests.
- **Checked by hand on this machine:** `cargo auditable build` of
  `codeagent` gives a binary with a 4.5 KB `.dep-v0` section. The tag
  gate and `cargo auditable zigbuild` were not run in a pipeline; that
  check is in `todo.md`.

