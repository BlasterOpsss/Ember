# Ember program (Step 2 scaffold)

## Run locally
1. Install Rust, Solana CLI (1.18+ / Agave) and Anchor 0.30.1 (`avm install 0.30.1 && avm use 0.30.1`).
2. `cd ember-program && yarn install`
3. `anchor keys sync` (replaces the placeholder program id in lib.rs and Anchor.toml)
4. `cargo test -p ember` runs the fee and curve unit tests (no validator needed).
5. `anchor test` builds and runs tests/ember.ts on a local validator.

If the build fails, send me the first error and I'll fix it.
