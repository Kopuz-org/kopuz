# Contributing to Kopuz

<!--toc:start-->

- [Contributing to Kopuz](#contributing-to-kopuz)
  - [Code of Conduct](#code-of-conduct)
  - [Before You Start](#before-you-start)
  - [Development Setup](#development-setup)
  - [Common Commands](#common-commands)
  - [Testing Expectations](#testing-expectations)
  - [Code Style](#code-style)
  - [Pull Requests and Submitting Changes](#pull-requests-and-submitting-changes)
    - [History Hygiene](#history-hygiene)
    - [Commit Messages](#commit-messages)
  - [Maintainer Communication](#maintainer-communication)
  - [AI Policy](#ai-policy)
    - [What This Means](#what-this-means)
  - [License](#license)

<!--toc:end-->

Kopuz is a Rust and Dioxus music player with desktop, packaging, and media
backend code living in one workspace. Contributions are welcome when they are
small enough to review, tested against the path they touch, and honest about the
platforms they were checked on.

## Code of Conduct

Keep project discussion practical and respectful. Bug reports, reviews, and
feature discussions should focus on the behavior of Kopuz and the code needed to
improve it. Do not harass contributors, demand unpaid support, or turn review
threads into personal arguments.

## Before You Start

Check the existing issues and pull requests before starting a larger change. For
small fixes, a pull request is fine. For new playback backends, database
changes, packaging changes, or UI rewrites, open an issue first so maintainers
can confirm the direction before review time is spent.

Good bug reports include:

- the Kopuz version or commit;
- the operating system and package format;
- the backend involved, such as local files, Jellyfin, Subsonic, YouTube Music,
  SoundCloud, ListenBrainz, or yt-dlp;
- relevant logs from **Settings -> Logs -> Export logs** when the bug involves
  playback, scanning, crashes, or network behavior.

## Development Setup

Nix is the preferred development environment because it provides the native
desktop libraries Kopuz needs:

```bash
# Enter a dev shell wtih the necessary dependencies
$ nix develop
```

If you use direnv:

```bash
# Allow Direnv to load the shell
$ direnv allow
```

For non-Nix systems, follow the dependency list in `README.md`. You will need a
Rust toolchain matching `rust-toolchain.toml`, Dioxus CLI 0.7.x, Node/npm for
Tailwind generation, and the platform libraries used by the desktop shell.

## Common Commands

```bash
npm install
just serve
just build
cargo test --workspace
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
nix build .#checks.x86_64-linux.default
```

Use `just serve` for normal desktop development; it regenerates Tailwind CSS and
runs Dioxus. Use `just build` when checking release packaging paths. If your
change touches translations, run:

```bash
# You can run the scripts with Nushell from Nixpkgs
$ nix shell nixpkgs#nushell --command nu scripts/check_locales.nu
$ nix shell nixpkgs#nushell --command nu scripts/check_i18n_usage.nu
```

## Testing Expectations

Run the smallest useful verifier before opening a pull request, then say what
you ran in the PR description. Examples:

- database query or migration changes: `cargo test -p kopuz-db`;
- playback or DSP changes: `cargo test -p kopuz-player`;
- shared server/source behavior: `cargo test -p kopuz-server`;
- UI or cross-crate changes: `cargo test --workspace` plus `just serve`;
- Nix packaging changes: `nix build .#checks.x86_64-linux.default`;
- macOS, Windows, Android, iOS, Flatpak, AUR, or AppImage work: test that target
  when possible and state clearly when you could not.

Do not silently skip tests that should cover your change. If a test cannot run
on your machine, explain the blocker.

## Code Style

Kopuz uses Rust 2024 and workspace lints from `Cargo.toml`.

- Prefer the existing crate boundaries under `crates/` over new shared layers.
- Keep UI code in the Dioxus style already used by `crates/components`,
  `crates/pages`, and `crates/kopuz`.
- Keep media-source behavior behind the existing source/provider abstractions
  instead of hardcoding one service into unrelated UI.
- Use `tracing` for diagnostics. Workspace Clippy denies `println!` and
  `eprintln!` outside explicit exceptions.
- Avoid holding Dioxus signal borrows across `.await`; `.clippy.toml` treats
  those types as invalid across await points.
- Prefer real error handling over `unwrap()` and `expect()` outside tests.
- Keep generated assets such as Tailwind output in sync when your change depends
  on them.

## Pull Requests and Submitting Changes

Keep commits and pull requests focused on one behavior change. A useful PR
description includes:

- what changed;
- how it was tested;
- screenshots or short clips for visible UI changes;
- logs or reproduction steps for playback, scanning, source, or crash fixes;
- any platform you could not test.

Maintainers may ask for narrower diffs, clearer tests, or a different boundary.
Please handle review comments in follow-up commits instead of force-pushing away
review context unless a maintainer asks you to clean up the branch.

### History Hygiene

Keep the final history reviewable; each commit should be an atomic, squashed
logical change with a useful subject. Before asking for final review, squash
fixup commits, typo-only follow-ups, formatter-only repair commits, and
review-addressing noise into the commit that introduced the behavior.

Avoid merge commits in pull request branches. Rebase on the target branch when
needed, then force-push with lease. A pull request may contain more than one
commit when the changes are genuinely separate, but each commit should build on
its own idea and pass the commit message rules below.

### Commit Messages

[scoped commits]

Please use clear, descriptive commit messages that adhere to **[scoped
commits]** format that are **no longer than 80 characters**. Since Kopuz is
rather large, the scope may differ. The main difference is how the scope is
described. Depending on your change, the scope may be the crate you are working
on or the crate plus the Rust module you've edited. For example, a change that
affects the entirety of the `components` crate would use the `components:`
prefix while a change targeting `header.rs` in `components` would use
`components/header`:

**One targeted module**:

```commitmsg
components/header: fix expanded state not persisting across track changes
```

**Various modules in one crate**:

```commitmsg
player: fix gapless playback gap calculation
```

**Multiple crates**:

```commitmsg
various: flush source state on library rescan
```

In single-crate commits, the format is `<crate>/<module>: <description>`. Some
common non-crate scopes include `various`, `treewide`, `chore` or `meta` for
common maintenance tasks. `docs` is the default scope for documentation changes,
and `build` can be used for the catch-all scope for packaging related changes.
We also use `nix` specifically for Nix-related packaging.

In most cases, the format will be `<crate-name>/<module-name>: <description>`.
You should also strive to make your commits atomic, and focus each commit on one
logical change. Ensure that your messages are _descriptive_ .

CI enforces these commit subject rules on pull requests and pushes:

- the subject must be no longer than 80 characters;
- the subject must use `scope: description` or `scope/module: description`.

> [!TIP]
> You do not need to reference issues in commit bodies, but you _may_ add
> something like `Fixes #XXX` to the long description to auto-link relevant
> issues.

## Maintainer Communication

Use GitHub issues for bugs and feature proposals, and pull request comments for
review. The Discord linked in `README.md` is fine for quick discussion, but
decisions that affect code should be copied back to an issue or pull request.

Do not privately ping maintainers for review unless they asked you to. If a pull
request has gone quiet for a while, leave one short public comment with the
current status and the checks you believe are still relevant.

## AI Policy

> [!IMPORTANT]
> Pull requests created or submitted by autonomous or supervised AI agents are
> explicitly prohibited, and will be immediately closed without a review. Kopuz,
> as a codebase, **does not welcome AI-generated contributions**.

This policy exists for the following reasons:

1. **Quality Assurance**: AI-generated code often lacks the contextual
   understanding required for systems-level software that interfaces with
   critical system components.

2. **Security**: Kopuz is used by various users on a daily basis. It runs in the
   foreground or the background during the time the user is consuming media, and
   has a relatively large attack surface unless fully sandboxed. Changes to such
   software require human judgment, security awareness, and accountability.

3. **Maintenance Burden**: AI-generated contributions often require
   disproportionate maintainer effort to review, correct, and integrate
   properly.

### What This Means

- **Prohibited**: Submitting PRs where an AI agent (autonomous or supervised)
  generated the code, commit messages, or PR description, regardless of whether
  a human clicked the "submit" button.

- **Prohibited**: Using AI agents to automatically fix issues, respond to review
  comments, or generate follow-up commits.

- **Allowed**: Using AI tools as aids while writing code, provided a human
  author thoroughly reviews, tests, and takes full responsibility for the
  submission. AI-assisted PRs require **FULL DISCLOSURE** and appropriate proof
  that the user thoroughly understands the code generated.

- **Allowed**: Using AI assistance for menial labor, e.g., _moving files_ that
  can be individually attested by the operator. Things like refactoring modules,
  removing or rewriting functions & structs, large-scale refactors that require
  attention MAY NOT be by AI agents.

By submitting a pull request, you attest that:

1. You are a human contributor
2. You have personally authored or thoroughly reviewed and tested all changes
   during and after AI influence
3. You take full moral, legal, and ethical responsibility for the contribution
4. No autonomous or supervised AI agent was used to create or submit the PR
5. Any AI assistance is thoroughly disclosed
6. You understand the consequences of violating above guidelines.

Violations of this policy may result in a permanent ban from contributing to the
project.

## License

By contributing to Kopuz, you agree that your contribution is licensed under the
MIT license used by this repository.
