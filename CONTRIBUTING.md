# Contributing to LinkScope

Thanks for helping. Bug reports, fixes, documentation (English or Bahasa) and tests are all
welcome. For anything larger than a fix, open an issue first so we can agree on the approach.

A security problem goes to a private report, not an issue: see
[docs/SECURITY.md](docs/SECURITY.md#reporting-a-vulnerability).

## The rules LinkScope keeps

- **MRTG compatibility.** An existing `mrtg.cfg` keeps working, and normal `.log` files stay
  byte-identical to MRTG 2.17.10's. A change that alters either needs a reason in the pull
  request and an entry under "Upgrading" in `CHANGELOG.md`.
- **No `unsafe`** in the parser, the evaluator and the rate engine.
- **Device data is hostile.** Anything read from an agent or a configuration is bounded and
  checked before use; secrets never reach the API, the logs or the UI unredacted.

## Building and checking

Rust 1.89 or newer (`rust-version` in `Cargo.toml`). Before you open a pull request:

```sh
cargo fmt --all
cargo clippy --all-targets --no-deps -p linkscope-config -p linkscope-rate -p linkscope-web -p linkscope -- -D warnings
cargo test --workspace
```

CI runs the same on every pull request, plus a build on the oldest supported Rust and a rebuild
of the UI. All must pass before anything merges into `main`.

### The web UI

The UI (React, in `crates/web/ui`) is built into `crates/web/ui/dist`, which is committed and
embedded in the binary, so building LinkScope needs no Node.js. When you change the UI or its
packages, rebuild it with Node.js 22 and commit `dist` with your change:

```sh
cd crates/web/ui
npm ci
npm run build
```

CI rebuilds the UI from the lock file and fails when the result differs from the committed
`dist`. That includes Dependabot's npm updates: they need the same rebuild on their branch.

### The manual

The manual is Markdown in `docs/manual/en` and `docs/manual/id` (Bahasa), with the same pages in
both. LinkScope serves it at `/docs`. Change both languages together when you can; if you can only
write one, say so in the pull request.

## Pull requests

- One change per pull request, with a description of what changed and why.
- Tests for what you fix or add: a failing test first is the best proof of a bug.
- Changes users notice go into `CHANGELOG.md`, under "Unreleased".

## License

LinkScope is GPL-2.0-or-later. By sending a contribution you agree that it is distributed under
the same license.
