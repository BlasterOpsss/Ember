# Ember program (Step 2 scaffold)

## Run locally
1. Install Rust, Solana CLI (1.18+ / Agave) and Anchor 0.30.1 (`avm install 0.30.1 && avm use 0.30.1`).
2. `cd ember-program && yarn install`
3. `anchor keys sync` (replaces the placeholder program id in lib.rs and Anchor.toml)
4. `cargo test -p ember` runs the fee and curve unit tests (no validator needed).
5. `anchor test` builds and runs tests/ember.ts on a local validator.

If the build fails, send me the first error and I'll fix it.

## Voting and treasury rules
- Voting locks the tokens in escrow. Taking them back out (`withdraw_vote`), during the vote or after it, cancels the ballot. A cancelled ballot cannot claim from the treasury.
- Only wallets that voted share in the treasury. A distribution divides the snapshot by the total weight of live ballots (`Vote.voter_weight`), and `claim_reward` needs the claimant's ballot. Claim before you withdraw.
