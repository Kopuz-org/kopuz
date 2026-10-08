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
    - [The Bar](#the-bar)
    - [Agents](#agents)
    - [Disclosure](#disclosure)
    - [Enforcement](#enforcement)
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
  SoundCloud, ListenBrainz, or YouTube downloads;
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

Run the smallest useful verifier before opening a pull request, then fill in the
Testing matrix in the pull request template. Examples:

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

Each pull request changes one behavior, however many crates it spans. A
refactor the change needs goes in its own pull request lower in a [stack], and
unrelated cleanups go in their own pull requests. If a pull request bundles
several changes, a maintainer will ask you to split it into a stack.

[stack]: https://github.github.com/gh-stack/

Fill in the pull request template as given. Its sections are fixed: do not add,
remove or rename headings, and do not add subheadings. Open the pull request as
a draft, and mark it ready for review once one column of the Testing matrix is
all ✅. For playback, scanning, source, or crash fixes, put the logs or
reproduction steps under Why.

Maintainers may ask for narrower diffs, clearer tests, or a different boundary.
Address review in follow-up commits if you like, but never undo earlier work
with a revert commit: rewrite the branch and force-push with lease, then squash
before final review as described below.

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

Kopuz accepts issues and pull requests written with LLMs or agents, as long as
they meet the same bar as every other contribution. The bar exists because
AI-written contributions tend to arrive large, unverified and wordy, and the
review cost lands on maintainers.

### The Bar

Every issue and pull request:

- comes from a human who asked for it and stands behind it;
- reads like a person wrote it: lead with the problem and the change in a few
  sentences, describe behavior instead of walking through the diff file by
  file, and leave out anything a reviewer does not need;
- includes a screenshot or recording for anything you can see or hear, such as
  UI and playback audio bugs.

Pull requests also:

- change one behavior (see
  [Pull Requests and Submitting Changes](#pull-requests-and-submitting-changes));
- fill in the Testing matrix with one row per changed behavior, and are ready
  for review once one platform column is all ✅;
- include a Before / After table for anything visible or audible, captured in
  the default dark theme with the same view and steps in both, or say "No
  visible or audible change";
- have been reviewed by a human who can explain every change.

### Agents

- An agent opens an issue or pull request only when a human explicitly asks
  for it, and opens pull requests as drafts.
- An agent makes follow-up commits only when a human asks for them.
- An agent marks a Testing cell ✅ only for a check it ran, with the screenshot
  or recording attached.
- Agents never reply to review comments; the human does.

### Disclosure

Tick the AI usage box in the template when you used an LLM to write code or
text. Ticking it does not change how the contribution is reviewed.

Do not add AI tools as `Co-authored-by:` trailers, and do not leave "Generated
with …" footers in commits, issues or pull requests; the checkbox is the only
disclosure. CI rejects pull request commits that carry them. The
`coderabbitai[bot]` trailer GitHub adds when you accept a review suggestion is
fine.

### Enforcement

Contributions that clearly miss the bar are closed without review, with a link
to this section. Near misses get a request for the missing parts. CodeRabbit
checks pull requests against this policy and warns about what is missing.

By submitting a pull request, you attest that:

1. A human asked for this contribution and takes full moral, legal, and ethical
   responsibility for it
2. A human reviewed every change and can explain it
3. Any LLM use is marked in the AI usage checkbox
4. You understand the consequences of violating above guidelines.

Violations of this policy may result in a permanent ban from contributing to the
project.

## License

By contributing to Kopuz, you agree that your contribution is licensed under the
European Union Public Licence v. 1.2 (EUPL-1.2) used by this repository.
