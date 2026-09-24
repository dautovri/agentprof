# Releasing

1. Update `version` in `Cargo.toml` and run `cargo build` to refresh
   `Cargo.lock`.
2. In `CHANGELOG.md`, rename `[Unreleased]` entries into a new
   `## [X.Y.Z] - YYYY-MM-DD` section and update the compare links.
3. Commit, then tag and push:

   ```bash
   git tag vX.Y.Z
   git push origin vX.Y.Z
   ```

The [Release workflow](.github/workflows/release.yml) then:

- checks that the tag matches `Cargo.toml`,
- builds macOS (arm64, x86_64) and static Linux (x86_64, arm64) binaries,
- publishes them with `.sha256` files and `SHA256SUMS`, using the
  changelog section as release notes,
- attaches a generated Homebrew formula (`agentprof.rb`), and pushes it to
  `dautovri/homebrew-tap` when a `HOMEBREW_TAP_TOKEN` secret is set.

To rebuild an existing tag, run the workflow manually with that tag.

## GitHub Marketplace

The repository root contains `action.yml`. To list the Action, edit the
release on GitHub and tick **Publish this Action to the GitHub Marketplace**.

## crates.io

The crate name `agentprof` is already taken on crates.io by an unrelated
project, so `cargo install agentprof` installs that project, not this one.
Publishing requires choosing another package name (the binary can stay
`agentprof` through a `[[bin]]` section).
