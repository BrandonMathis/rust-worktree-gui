# Security

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub: **Security → Report a vulnerability** on this repository. Don't open a public issue for them.

## Dependency updates

The main supply-chain risk for this app is a dependency that ships malicious code, often after an attacker takes over a maintainer account and publishes a quiet patch release. The update policy is built around that.

| Update | What happens | Why |
| --- | --- | --- |
| Patch release (`1.2.3 → 1.2.4`) | **No Dependabot PR.** | Patches are what `cargo update` and loose requirements pull in silently, so they're the favorite hiding place for a hijacked release. |
| Patch release that fixes an advisory (CVE/GHSA/RustSec) | **Dependabot security update PR**, opened right away. | A known vulnerability outweighs the risk of an unvetted patch. These PRs still go through review and CI. |
| Minor or major release | Dependabot version update PR every Monday, only once the release is **14 days old** (30 for majors). | Most malicious releases are reported and yanked within days. The cooldown never delays security updates. |
| GitHub Actions | Pinned to full commit SHAs. Dependabot proposes minor and major bumps after 14 days, in one grouped PR. | A moved tag can't change what CI runs. |

How this is configured (`.github/dependabot.yml`):

- Version updates use `allow: update-types: [semver-minor, semver-major]`. GitHub documents that `allow`'s `update-types` "only affects version updates, not security updates", so CVE patch releases still arrive.
- Don't rewrite this as `ignore: update-types: [version-update:semver-patch]`. `ignore` rules also apply to security updates, so that version would block CVE fixes.
- `iced_term` is excluded because it is vendored and patched in `vendor/iced_term`.
- Security updates are a repository setting: **Settings → Advanced Security → Dependabot alerts** and **Dependabot security updates** must both be on.

Patch releases still arrive in two ways: through a security update, or as part of a deliberate `cargo update -p <crate>`, which you then review in `Cargo.lock`. CI always builds with `--locked`, so nothing moves without a commit.

## Agent SDKs (npm)

Managed worktrees run a Node.js sidecar (`agent-bridge/`) that uses three npm packages: `@anthropic-ai/claude-agent-sdk`, `@openai/codex-sdk` and `@earendil-works/pi-coding-agent` (with `@earendil-works/pi-ai`). They are the only npm dependencies, and they follow the same policy as the crates:

- `package.json` pins exact versions, and `package-lock.json` records every transitive package with its checksum. The lockfile is compiled into the app.
- Nothing is installed until the user clicks **Install Agent SDKs**. The app then runs `npm ci --omit=dev --ignore-scripts` against that lockfile: exactly the locked versions, checksums verified, and no package install scripts run.
- Dependabot covers `/agent-bridge` with the same rules as Cargo: minor and major updates after the cooldown, patches only as security updates.
- The SDKs run coding agents that execute commands in the worktree by design. What each may do without asking is in `docs/frd/FRD-10-managed-worktrees.md` (MGD-12). Links in an agent's reply are opened only if they are plain `https://` URLs.

## Automated checks

| Check | Where | When | Guards against |
| --- | --- | --- | --- |
| `cargo deny check` (advisories, licenses, bans, sources) | `ci.yml` | Every PR and push to `main` | Vulnerable, unmaintained or yanked crates; disallowed licenses; crates from unknown registries or git repos |
| `cargo deny check advisories` | `security.yml` | Daily | Advisories published after a dependency was merged. A failure opens an issue labelled `security-advisory`. |
| Dependency review | `security.yml` | Every PR | A PR that adds a dependency with a known vulnerability (fails at any severity), or one with a low OpenSSF Scorecard rating (warning) |
| CodeQL (`security-extended`) | `security.yml` | Every PR, push to `main`, daily | Security bugs in our Rust code, such as path or command injection |
| zizmor | `security.yml` | Every PR and push to `main` | Workflow injection, leaked credentials, over-broad permissions, unpinned actions |
| Dependency health report | `dependency-health.yml` | Mondays | Direct dependencies that are majorly out of date or out of support, and an aged-out Rust toolchain (see below) |
| Clippy with `-D warnings` and panic lints | `ci.yml` | Every PR | Panics on I/O or subprocess results (see `Cargo.toml` `[lints]`) |

Code scanning results (CodeQL, zizmor) appear under **Security → Code scanning**.

## Dependency health report

`scripts/dependency_health.rb` uses only the Ruby standard library. It reads `Cargo.lock` through `cargo metadata`, looks up each direct dependency on crates.io, and reports:

- **Majorly out of date:** two or more breaking release lines behind (`1.x → 3.x`, or `0.14 → 0.16` for pre-1.0 crates), or one breaking line behind for 180+ days.
- **Out of support:** the locked version is yanked, or the crate has had no release for 3 years. Crates quiet for 18 months go on a watch list instead, because small, finished crates are often quiet.
- **Toolchain:** the pinned Rust (`.tool-versions`) is 4+ releases behind stable. Rust has no LTS, so only the latest stable gets fixes.
- **Accepted advisories:** the RustSec IDs that `deny.toml` ignores, so accepted risks stay visible.

While there are findings, one issue labelled `dependency-health` stays open. Each run that changes the report adds a comment, which notifies watchers, and the issue closes itself once the report is clean. Thresholds are constants at the top of the script.

Unmaintained *transitive* crates are caught by `cargo deny` through RustSec "unmaintained" advisories, not by this report.

```sh
ruby scripts/dependency_health.rb              # run the report locally
ruby scripts/test/dependency_health_test.rb    # unit tests for its rules
```
