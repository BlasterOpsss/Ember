use anchor_lang::prelude::*;

#[error_code]
pub enum EmberError {
    #[msg("Lifespan must be 24 hours, 7 days or 30 days")]
    BadLifespan,
    #[msg("Total fee must be 1% to 5%, including the 1% platform fee")]
    BadFee,
    #[msg("Treasury share must be 25% to 100% of the creator fee")]
    BadTreasuryShare,
    #[msg("Quorum must be 20% to 50%")]
    BadQuorum,
    #[msg("Vote mode must be 0 (token weighted) or 1 (one wallet one vote)")]
    BadVoteMode,
    #[msg("Coin is not live")]
    NotLive,
    #[msg("Coin has not reached its end time")]
    NotEnded,
    #[msg("Coin has ended")]
    Ended,
    #[msg("Coin is dead")]
    Dead,
    #[msg("Coin is not in voting")]
    NotVoting,
    #[msg("Voting window is closed")]
    VoteClosed,
    #[msg("Voting window is still open")]
    VoteStillOpen,
    #[msg("Vote already finalized")]
    AlreadyFinalized,
    #[msg("This wallet already has an active vote")]
    AlreadyVoted,
    #[msg("No active vote for this wallet")]
    NotVoted,
    #[msg("Vote belongs to another life of this coin")]
    WrongLife,
    #[msg("Coin must be closed or dead to redeem")]
    CannotRedeem,
    #[msg("Slippage limit hit")]
    Slippage,
    #[msg("Math overflow")]
    Math,
    #[msg("Nothing to claim")]
    NothingToClaim,
    #[msg("Not the creator")]
    NotCreator,
    #[msg("Amount must be above zero and within your balance")]
    BadAmount,
    #[msg("Day does not match the current day")]
    WrongDay,
    #[msg("Check-ins are closed in the final hour of a life")]
    FinalWindow,
    #[msg("Hold at least 0.1% of supply to check in")]
    BelowMinHold,
    #[msg("That day has not ended yet")]
    DayNotEnded,
    #[msg("Days must be settled in order")]
    WrongSettleDay,
    #[msg("That day is after the end of this life")]
    DayAfterEnd,
    #[msg("Treasury payouts are not enabled yet")]
    PayoutsDisabled,
    #[msg("Only the admin can do this")]
    NotAdmin,
    #[msg("This vote cannot open a distribution")]
    NotEligible,
    #[msg("Already claimed")]
    AlreadyClaimed,
    #[msg("Nothing to pay for this wallet")]
    NothingToPay,
    #[msg("The claim window has closed")]
    ClaimWindowClosed,
    #[msg("The claim window is still open")]
    ClaimWindowOpen,
    #[msg("This ballot was cancelled, so the wallet cannot share in the treasury")]
    VoteCancelled,
    #[msg("Already swept")]
    AlreadySwept,
}
