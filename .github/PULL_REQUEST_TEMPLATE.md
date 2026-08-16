<!--
Please include a clear and concise description of the aim of your pull request
above this line.

If this pull request fixes an open issue, link it with `Fixes #<issue number>`.
For playback, scanning, source, packaging, or crash fixes, include the relevant
logs or reproduction steps.
-->

## Sanity Checking

<!--
Please check all that apply. These boxes help maintainers quickly understand
what you already verified and what still needs review.
-->

[contribution guidelines]: https://github.com/Kopuz-org/kopuz/blob/master/CONTRIBUTING.md

- [ ] I have read and followed the [contribution guidelines].
- [ ] My commits follow Kopuz's scoped commit convention and history hygiene
      rules.
- [ ] I have disclosed any AI assistance as required by the AI policy in the
      [contribution guidelines], or this pull request did not use AI assistance.
- [ ] I have tested and self-reviewed my changes.

### Style and Consistency

- [ ] My changes are consistent with the existing crate boundaries and Dioxus
      style.
- [ ] I ran `cargo fmt --all --check` or `cargo fmt --all` as appropriate.
- [ ] I ran `cargo clippy --workspace --all-targets -- -D warnings`, or
      explained why it could not be run.
- [ ] I kept generated assets, translations, and packaging files in sync when
      this change depends on them.

### Testing

- [ ] I ran the smallest relevant verifier for this change.
- [ ] I documented any platform or verifier that I could not run.

Tested on platform(s):

- [ ] `x86_64-linux`
- [ ] `aarch64-linux`
- [ ] `x86_64-darwin`
- [ ] `aarch64-darwin`
- [ ] Windows
- [ ] Android
- [ ] iOS
