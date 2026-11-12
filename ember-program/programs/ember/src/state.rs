use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct Config {
    pub admin: Pubkey, // multisig
    pub platform_wallet: Pubkey,
    pub bump: u8,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum CoinStatus {
    Live,
    Voting,
    Dead,
}

/// Locked at launch. Quorum is always measured in tokens; the mode only
/// changes how the majority is counted.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum VoteMode {
    Token,  // each locked token is one vote
    Wallet, // each wallet that locks the minimum is one vote
}

#[account]
#[derive(InitSpace)]
pub struct Coin {
    pub creator: Pubkey,
    pub mint: Pubkey,
    pub status: CoinStatus,
    pub vote_mode: VoteMode,
    pub life_index: u8,
    pub start_ts: i64,
    pub end_ts: i64,
    // Locked at creation, never changed.
    pub lifespan_secs: i64,
    pub total_fee_bps: u16,
    pub treasury_share_bps: u16,
    pub quorum_bps: u16,
    // Curve. Invariants: virtual_sol - real_sol == INIT_VIRTUAL_SOL and
    // virtual_token == tokens held by the curve vault.
    pub virtual_sol: u64,
    pub virtual_token: u64,
    pub real_sol: u64,
    pub creator_fees_claimable: u64,
    // Community streak (settled day by day).
    pub community_streak: u16,
    pub best_community_streak: u16,
    pub next_settle_day: u32,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct WalletState {
    pub bought_total: u64,
    pub first_buy_ts: i64,
    pub streak_days: u16,
    pub best_streak: u16,
    pub last_checkin_day: u32,
    pub total_checkins: u32,
}

/// Marker that a wallet checked in on a day. Existing = already checked in.
/// Seeds: ["checkin", coin, life_index, day_le, wallet].
#[account]
#[derive(InitSpace)]
pub struct CheckInRecord {
    pub tokens: u64,
}

/// Seeds: ["day", coin, life_index, day_le].
#[account]
#[derive(InitSpace)]
pub struct DayStats {
    pub coin: Pubkey,
    pub day: u32,
    pub checked_tokens: u64, // sum of balances of wallets that checked in
    pub checkins: u32,
    pub settled: bool,
    pub passed: bool,
}

/// One per coin per life. Seeds: ["vote", coin, life_index].
#[account]
#[derive(InitSpace)]
pub struct Vote {
    pub coin: Pubkey,
    pub life_index: u8,
    pub end_ts: i64,
    pub circulating_snapshot: u64, // supply held outside the curve at close
    pub locked_tokens: u64,        // turnout for quorum
    pub revive_yes_tokens: u64,
    pub revive_no_tokens: u64,
    pub release_yes_tokens: u64,
    pub release_no_tokens: u64,
    pub revive_yes_wallets: u32,
    pub revive_no_wallets: u32,
    pub release_yes_wallets: u32,
    pub release_no_wallets: u32,
    pub finalized: bool,
    pub revived: bool,
    pub release_passed: bool, // recorded only; treasury payout is built later
    pub bump: u8,
}

/// One per wallet per vote. Seeds: ["voted", vote, wallet].
#[account]
#[derive(InitSpace)]
pub struct VoteRecord {
    pub amount: u64,
    pub revive_yes: bool,
    pub release_yes: bool,
    pub counts_wallet: bool,
    pub active: bool,
}
