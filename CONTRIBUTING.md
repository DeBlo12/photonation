# Contributing to Photonation

Thanks for helping! Bug reports, camera samples, ideas and pull requests are all
welcome. Please follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Reporting bugs and requesting features

Use the issue templates. For rendering bugs, include the camera model, the
`photonation info <file>` output and, if you can share it, the raw file
(it will be public).

## Development setup

```bash
git clone https://github.com/DeBlo12/photonation.git
```

```bash
cargo test --workspace
```

The toolchain is pinned in `rust-toolchain.toml`; rustup installs it
automatically. See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for the
architecture, conventions and how to add a control.

## Pull requests

1. Branch from `main` (`feature/short-name`, `fix/short-name`).
2. Keep changes focused; update the matching document in `docs/` and add an
   entry to [CHANGELOG.md](CHANGELOG.md) under *Unreleased* for user-visible
   changes.
3. Before pushing, run:

   ```bash
   cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
   ```

4. New dependencies must use a licence allowed in `deny.toml`
   (`cargo deny check`).
5. Open the pull request and fill in the template; CI must pass.

## Licensing of contributions

Photonation is licensed under the GNU GPL v3.0 or later. By submitting a
contribution you agree that it is licensed under the same terms
(inbound = outbound), and you confirm that you have the right to submit it.
Please add the SPDX header to new source files:

```rust
// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 The Photonation contributors
```
