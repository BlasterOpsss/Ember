use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer, Transfer};
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token_2022::spl_token_2022::instruction::AuthorityType;
use anchor_spl::token_interface::{
    burn, mint_to, set_authority, transfer_checked, Burn, Mint, MintTo, SetAuthority,
    TokenAccount, TokenInterface, TransferChecked,
};

pub mod errors;
pub mod math;
pub mod state;
use errors::EmberError;
use math::*;
use state::*;

declare_id!("ELF3WhvZLbFm9wA2NeWp1e7tn3KCxwWEsXVC3M8oim5y"); // replace after `anchor keys sync`

pub const DECIMALS: u8 = 6;
pub const TOTAL_SUPPLY: u64 = 1_000_000_000 * 1_000_000; // 1B tokens
pub const INIT_VIRTUAL_SOL: u64 = 30_000_000_000; // 30 SOL
#[cfg(not(feature = "short-life"))]
pub const LIFESPANS: &[i64] = &[86_400, 604_800, 2_592_000]; // 24h, 7d, 30d
#[cfg(feature = "short-life")]
pub const LIFESPANS: &[i64] = &[3, 40, 604_800, 2_592_000]; // TEST BUILDS ONLY
#[cfg(not(feature = "short-life"))]
pub const DAY_SECS: i64 = 86_400;
#[cfg(feature = "short-life")]
pub const DAY_SECS: i64 = 6; // TEST BUILDS ONLY
#[cfg(not(feature = "short-life"))]
pub const FINAL_WINDOW_SECS: i64 = 3_600; // no check-ins in the last hour
#[cfg(feature = "short-life")]
pub const FINAL_WINDOW_SECS: i64 = 1; // TEST BUILDS ONLY
pub const MIN_CHECKIN_HOLD: u64 = TOTAL_SUPPLY / 1_000; // 0.1% of supply
pub const COMMUNITY_THRESHOLD_BPS: u128 = 3_000; // 30% of circulating supply
#[cfg(not(feature = "short-life"))]
pub const VOTE_SECS: i64 = 86_400;
#[cfg(feature = "short-life")]
pub const VOTE_SECS: i64 = 8; // TEST BUILDS ONLY
pub const MIN_WALLET_LOCK: u64 = TOTAL_SUPPLY / 1_000; // 0.1% of supply, wallet mode

fn pay_from_user<'info>(
    sys: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    amt: u64,
) -> Result<()> {
    if amt == 0 {
        return Ok(());
    }
    transfer(
        CpiContext::new(sys.clone(), Transfer { from: from.clone(), to: to.clone() }),
        amt,
    )
}

fn pay_from_vault<'info>(
    sys: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    seeds: &[&[u8]],
    amt: u64,
) -> Result<()> {
    if amt == 0 {
        return Ok(());
    }
    transfer(
        CpiContext::new_with_signer(
            sys.clone(),
            Transfer { from: from.clone(), to: to.clone() },
            &[seeds],
        ),
        amt,
    )
}

fn day_of(ts: i64) -> u32 {
    (ts / DAY_SECS) as u32
}

fn bump64(x: &mut u64, d: u64, add: bool) -> Result<()> {
    *x = if add { x.checked_add(d) } else { x.checked_sub(d) }.ok_or(EmberError::Math)?;
    Ok(())
}

fn bump32(x: &mut u32, add: bool) -> Result<()> {
    *x = if add { x.checked_add(1) } else { x.checked_sub(1) }.ok_or(EmberError::Math)?;
    Ok(())
}

/// Adds or removes one ballot from both the token and wallet tallies.
fn tally(
    v: &mut Vote,
    amount: u64,
    revive_yes: bool,
    release_yes: bool,
    counts_wallet: bool,
    add: bool,
) -> Result<()> {
    bump64(&mut v.locked_tokens, amount, add)?;
    if revive_yes {
        bump64(&mut v.revive_yes_tokens, amount, add)?;
        if counts_wallet {
            bump32(&mut v.revive_yes_wallets, add)?;
        }
    } else {
        bump64(&mut v.revive_no_tokens, amount, add)?;
        if counts_wallet {
            bump32(&mut v.revive_no_wallets, add)?;
        }
    }
    if release_yes {
        bump64(&mut v.release_yes_tokens, amount, add)?;
        if counts_wallet {
            bump32(&mut v.release_yes_wallets, add)?;
        }
    } else {
        bump64(&mut v.release_no_tokens, amount, add)?;
        if counts_wallet {
            bump32(&mut v.release_no_wallets, add)?;
        }
    }
    Ok(())
}

#[program]
pub mod ember {
    use super::*;

    pub fn initialize_config(ctx: Context<InitializeConfig>, platform_wallet: Pubkey) -> Result<()> {
        let c = &mut ctx.accounts.config;
        c.admin = ctx.accounts.admin.key();
        c.platform_wallet = platform_wallet;
        c.bump = ctx.bumps.config;
        Ok(())
    }

    /// Fee, treasury share, quorum and lifespan are locked here for good.
    pub fn create_coin(
        ctx: Context<CreateCoin>,
        lifespan_secs: i64,
        total_fee_bps: u16,
        treasury_share_bps: u16,
        quorum_bps: u16,
        vote_mode: u8,
    ) -> Result<()> {
        let mode = match vote_mode {
            0 => VoteMode::Token,
            1 => VoteMode::Wallet,
            _ => return err!(EmberError::BadVoteMode),
        };
        require!(LIFESPANS.contains(&lifespan_secs), EmberError::BadLifespan);
        require!(
            (PLATFORM_FEE_BPS..=MAX_TOTAL_FEE_BPS).contains(&(total_fee_bps as u64)),
            EmberError::BadFee
        );
        require!(
            (MIN_TREASURY_SHARE_BPS..=10_000).contains(&(treasury_share_bps as u64)),
            EmberError::BadTreasuryShare
        );
        require!(
            (MIN_QUORUM_BPS..=MAX_QUORUM_BPS).contains(&(quorum_bps as u64)),
            EmberError::BadQuorum
        );

        let now = Clock::get()?.unix_timestamp;
        let mint_key = ctx.accounts.mint.key();
        let bump = ctx.bumps.coin;
        {
            let coin = &mut ctx.accounts.coin;
            coin.creator = ctx.accounts.creator.key();
            coin.mint = mint_key;
            coin.status = CoinStatus::Live;
            coin.vote_mode = mode;
            coin.life_index = 1;
            coin.start_ts = now;
            coin.end_ts = now.checked_add(lifespan_secs).ok_or(EmberError::Math)?;
            coin.lifespan_secs = lifespan_secs;
            coin.total_fee_bps = total_fee_bps;
            coin.treasury_share_bps = treasury_share_bps;
            coin.quorum_bps = quorum_bps;
            coin.virtual_sol = INIT_VIRTUAL_SOL;
            coin.virtual_token = TOTAL_SUPPLY;
            coin.real_sol = 0;
            coin.creator_fees_claimable = 0;
            coin.community_streak = 0;
            coin.best_community_streak = 0;
            coin.next_settle_day = day_of(now);
            coin.bump = bump;
        }

        // Seed the three vault PDAs with rent-exempt minimum (system accounts).
        let rent = Rent::get()?.minimum_balance(0);
        for vault in [
            ctx.accounts.sol_vault.to_account_info(),
            ctx.accounts.treasury.to_account_info(),
            ctx.accounts.creator_vault.to_account_info(),
        ] {
            transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    Transfer { from: ctx.accounts.creator.to_account_info(), to: vault },
                ),
                rent,
            )?;
        }

        // Mint the whole supply into the curve vault, then burn the mint authority.
        let seeds: &[&[u8]] = &[b"coin", mint_key.as_ref(), &[bump]];
        mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.token_vault.to_account_info(),
                    authority: ctx.accounts.coin.to_account_info(),
                },
                &[seeds],
            ),
            TOTAL_SUPPLY,
        )?;
        set_authority(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                SetAuthority {
                    current_authority: ctx.accounts.coin.to_account_info(),
                    account_or_mint: ctx.accounts.mint.to_account_info(),
                },
                &[seeds],
            ),
            AuthorityType::MintTokens,
            None,
        )?;
        Ok(())
    }

    pub fn buy(ctx: Context<Trade>, sol_in: u64, min_tokens_out: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let (split, net, out, mint_key, coin_bump, coin_key) = {
            let coin = &mut ctx.accounts.coin;
            require!(coin.status == CoinStatus::Live, EmberError::NotLive);
            require!(now < coin.end_ts, EmberError::Ended);
            let split = split_fee(sol_in, coin.total_fee_bps as u64, coin.treasury_share_bps as u64)
                .ok_or(EmberError::BadFee)?;
            let net = sol_in.checked_sub(split.total()).ok_or(EmberError::Math)?;
            let out = tokens_out(coin.virtual_sol, coin.virtual_token, net).ok_or(EmberError::Math)?;
            require!(out > 0 && out >= min_tokens_out, EmberError::Slippage);
            require!(ctx.accounts.token_vault.amount >= out, EmberError::Math);

            let ws = &mut ctx.accounts.wallet_state;
            if ws.first_buy_ts == 0 {
                ws.first_buy_ts = now;
            }
            ws.bought_total = ws.bought_total.checked_add(out).ok_or(EmberError::Math)?;
    
            coin.virtual_sol = coin.virtual_sol.checked_add(net).ok_or(EmberError::Math)?;
            coin.virtual_token = coin.virtual_token.checked_sub(out).ok_or(EmberError::Math)?;
            coin.real_sol = coin.real_sol.checked_add(net).ok_or(EmberError::Math)?;
            coin.creator_fees_claimable =
                coin.creator_fees_claimable.checked_add(split.creator).ok_or(EmberError::Math)?;
            (split, net, out, coin.mint, coin.bump, ctx.accounts.coin.key())
        };

        let user = ctx.accounts.user.to_account_info();
        let sys = ctx.accounts.system_program.to_account_info();
        pay_from_user(&sys, &user, &ctx.accounts.sol_vault.to_account_info(), net)?;
        pay_from_user(&sys, &user, &ctx.accounts.platform_wallet.to_account_info(), split.platform)?;
        pay_from_user(&sys, &user, &ctx.accounts.treasury.to_account_info(), split.treasury)?;
        pay_from_user(&sys, &user, &ctx.accounts.creator_vault.to_account_info(), split.creator)?;

        let _ = coin_key;
        let seeds: &[&[u8]] = &[b"coin", mint_key.as_ref(), &[coin_bump]];
        transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.token_vault.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.user_token.to_account_info(),
                    authority: ctx.accounts.coin.to_account_info(),
                },
                &[seeds],
            ),
            out,
            DECIMALS,
        )
    }

    pub fn sell(ctx: Context<Trade>, tokens_in: u64, min_sol_out: u64) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let (split, payout, coin_key, sol_bump) = {
            let coin = &mut ctx.accounts.coin;
            require!(coin.status == CoinStatus::Live, EmberError::NotLive);
            require!(now < coin.end_ts, EmberError::Ended);
            let gross = sol_out(coin.virtual_sol, coin.virtual_token, tokens_in).ok_or(EmberError::Math)?;
            require!(coin.real_sol >= gross, EmberError::Math);
            let split = split_fee(gross, coin.total_fee_bps as u64, coin.treasury_share_bps as u64)
                .ok_or(EmberError::BadFee)?;
            let payout = gross.checked_sub(split.total()).ok_or(EmberError::Math)?;
            require!(payout > 0 && payout >= min_sol_out, EmberError::Slippage);

            coin.virtual_sol = coin.virtual_sol.checked_sub(gross).ok_or(EmberError::Math)?;
            coin.virtual_token = coin.virtual_token.checked_add(tokens_in).ok_or(EmberError::Math)?;
            coin.real_sol = coin.real_sol.checked_sub(gross).ok_or(EmberError::Math)?;
            coin.creator_fees_claimable =
                coin.creator_fees_claimable.checked_add(split.creator).ok_or(EmberError::Math)?;
            (split, payout, ctx.accounts.coin.key(), ctx.bumps.sol_vault)
        };

        // Tokens back into the curve vault (user signs).
        transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.user_token.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.token_vault.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            tokens_in,
            DECIMALS,
        )?;

        // SOL out of the vault PDA.
        let seeds: &[&[u8]] = &[b"sol_vault", coin_key.as_ref(), &[sol_bump]];
        let from = ctx.accounts.sol_vault.to_account_info();
        let sys = ctx.accounts.system_program.to_account_info();
        pay_from_vault(&sys, &from, &ctx.accounts.user.to_account_info(), seeds, payout)?;
        pay_from_vault(&sys, &from, &ctx.accounts.platform_wallet.to_account_info(), seeds, split.platform)?;
        pay_from_vault(&sys, &from, &ctx.accounts.treasury.to_account_info(), seeds, split.treasury)?;
        pay_from_vault(&sys, &from, &ctx.accounts.creator_vault.to_account_info(), seeds, split.creator)
    }

    /// Anyone can call this once the timer has passed. Trading stops and a
    /// 24 hour vote opens.
    pub fn close_coin(ctx: Context<CloseCoin>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let coin_key = ctx.accounts.coin.key();
        let coin = &mut ctx.accounts.coin;
        require!(coin.status == CoinStatus::Live, EmberError::NotLive);
        require!(now >= coin.end_ts, EmberError::NotEnded);
        coin.status = CoinStatus::Voting;

        let v = &mut ctx.accounts.vote;
        v.coin = coin_key;
        v.life_index = coin.life_index;
        v.end_ts = now.checked_add(VOTE_SECS).ok_or(EmberError::Math)?;
        v.circulating_snapshot =
            ctx.accounts.mint.supply.saturating_sub(ctx.accounts.token_vault.amount);
        v.bump = ctx.bumps.vote;
        Ok(())
    }

    /// Locks `amount` tokens in escrow and records one ballot with two
    /// questions: revive the coin, and release the treasury.
    pub fn cast_vote(
        ctx: Context<CastVote>,
        amount: u64,
        revive_yes: bool,
        release_yes: bool,
    ) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        require!(ctx.accounts.coin.status == CoinStatus::Voting, EmberError::NotVoting);
        require!(now < ctx.accounts.vote.end_ts, EmberError::VoteClosed);
        require!(!ctx.accounts.record.active, EmberError::AlreadyVoted);
        require!(
            amount > 0 && amount <= ctx.accounts.voter_token.amount,
            EmberError::BadAmount
        );

        transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.voter_token.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.escrow.to_account_info(),
                    authority: ctx.accounts.voter.to_account_info(),
                },
            ),
            amount,
            DECIMALS,
        )?;

        let counts_wallet = amount >= MIN_WALLET_LOCK;
        tally(&mut ctx.accounts.vote, amount, revive_yes, release_yes, counts_wallet, true)?;
        let rec = &mut ctx.accounts.record;
        rec.amount = amount;
        rec.revive_yes = revive_yes;
        rec.release_yes = release_yes;
        rec.counts_wallet = counts_wallet;
        rec.active = true;
        Ok(())
    }

    /// During the vote: returns the tokens and cancels the ballot (it is
    /// subtracted from the tally). After finalize: just returns the tokens.
    pub fn withdraw_vote(ctx: Context<WithdrawVote>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        require!(ctx.accounts.record.active, EmberError::NotVoted);
        let (amount, coin_key, life, bump) = {
            let rec = ctx.accounts.record.clone();
            let v = &mut ctx.accounts.vote;
            if !v.finalized {
                require!(now < v.end_ts, EmberError::VoteStillOpen);
                tally(v, rec.amount, rec.revive_yes, rec.release_yes, rec.counts_wallet, false)?;
            }
            (rec.amount, v.coin, v.life_index, v.bump)
        };
        ctx.accounts.record.active = false;

        let life_arr = [life];
        let bump_arr = [bump];
        let seeds: &[&[u8]] = &[b"vote", coin_key.as_ref(), &life_arr, &bump_arr];
        transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.escrow.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.voter_token.to_account_info(),
                    authority: ctx.accounts.vote.to_account_info(),
                },
                &[seeds],
            ),
            amount,
            DECIMALS,
        )
    }

    /// Anyone, once the vote window has passed. Revive needs the coin's
    /// quorum plus a simple majority. Release needs the quorum plus two thirds.
    pub fn finalize_vote(ctx: Context<FinalizeVote>) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let coin = &mut ctx.accounts.coin;
        let v = &mut ctx.accounts.vote;
        require!(coin.status == CoinStatus::Voting, EmberError::NotVoting);
        require!(!v.finalized, EmberError::AlreadyFinalized);
        require!(v.life_index == coin.life_index, EmberError::WrongLife);
        require!(now >= v.end_ts, EmberError::VoteStillOpen);

        let met = quorum_met(v.locked_tokens, v.circulating_snapshot, coin.quorum_bps);
        let (ry, rn, ly, ln) = match coin.vote_mode {
            VoteMode::Token => (
                v.revive_yes_tokens,
                v.revive_no_tokens,
                v.release_yes_tokens,
                v.release_no_tokens,
            ),
            VoteMode::Wallet => (
                v.revive_yes_wallets as u64,
                v.revive_no_wallets as u64,
                v.release_yes_wallets as u64,
                v.release_no_wallets as u64,
            ),
        };
        let revive = met && simple_majority(ry, rn) && coin.life_index < MAX_LIVES;
        let release = met && two_thirds(ly, ln);

        v.finalized = true;
        v.revived = revive;
        v.release_passed = release;
        if revive {
            // Same curve, new life.
            coin.life_index += 1;
            coin.status = CoinStatus::Live;
            coin.start_ts = now;
            coin.end_ts = now.checked_add(coin.lifespan_secs).ok_or(EmberError::Math)?;
            coin.community_streak = 0;
            coin.next_settle_day = day_of(now);
        } else {
            coin.status = CoinStatus::Dead;
        }
        Ok(())
    }

    /// Burn tokens for a pro rata share of the pool. Allowed while voting
    /// and after death. The curve is rescaled so the price does not jump.
    pub fn redeem(ctx: Context<Redeem>, amount: u64) -> Result<()> {
        require!(
            ctx.accounts.coin.status != CoinStatus::Live,
            EmberError::CannotRedeem
        );
        require!(
            amount > 0 && amount <= ctx.accounts.user_token.amount,
            EmberError::BadAmount
        );
        let circulating = ctx.accounts.mint.supply.saturating_sub(ctx.accounts.token_vault.amount);
        let (payout, vault_burn, mint_key, coin_bump, coin_key) = {
            let coin = &mut ctx.accounts.coin;
            let payout = redeem_payout(coin.real_sol, amount, circulating).ok_or(EmberError::Math)?;
            let vs_before = coin.virtual_sol;
            let vs_after = vs_before.checked_sub(payout).ok_or(EmberError::Math)?;
            let vt_after = scaled_virtual_token(coin.virtual_token, vs_after, vs_before)
                .ok_or(EmberError::Math)?;
            let vault_burn = coin.virtual_token.checked_sub(vt_after).ok_or(EmberError::Math)?;
            coin.real_sol = coin.real_sol.checked_sub(payout).ok_or(EmberError::Math)?;
            coin.virtual_sol = vs_after;
            coin.virtual_token = vt_after;
            (payout, vault_burn, coin.mint, coin.bump, ctx.accounts.coin.key())
        };

        burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.mint.to_account_info(),
                    from: ctx.accounts.user_token.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount,
        )?;
        if vault_burn > 0 {
            let coin_seeds: &[&[u8]] = &[b"coin", mint_key.as_ref(), &[coin_bump]];
            burn(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Burn {
                        mint: ctx.accounts.mint.to_account_info(),
                        from: ctx.accounts.token_vault.to_account_info(),
                        authority: ctx.accounts.coin.to_account_info(),
                    },
                    &[coin_seeds],
                ),
                vault_burn,
            )?;
        }
        let sol_seeds: &[&[u8]] = &[b"sol_vault", coin_key.as_ref(), &[ctx.bumps.sol_vault]];
        pay_from_vault(
            &ctx.accounts.system_program.to_account_info(),
            &ctx.accounts.sol_vault.to_account_info(),
            &ctx.accounts.user.to_account_info(),
            sol_seeds,
            payout,
        )
    }

    /// One check-in per wallet per day. The wallet must hold at least 0.1% of
    /// supply. Check-ins are closed in the final hour of a life.
    pub fn check_in(ctx: Context<CheckInCtx>, day: u32) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        require!(day == day_of(now), EmberError::WrongDay);
        let coin_key = ctx.accounts.coin.key();
        let end_ts = ctx.accounts.coin.end_ts;
        require!(ctx.accounts.coin.status == CoinStatus::Live, EmberError::NotLive);
        require!(
            now.checked_add(FINAL_WINDOW_SECS).ok_or(EmberError::Math)? < end_ts,
            EmberError::FinalWindow
        );
        let bal = ctx.accounts.user_token.amount;
        require!(bal >= MIN_CHECKIN_HOLD, EmberError::BelowMinHold);

        let ws = &mut ctx.accounts.wallet_state;
        if ws.total_checkins > 0 && ws.last_checkin_day.checked_add(1) == Some(day) {
            ws.streak_days = ws.streak_days.saturating_add(1);
        } else {
            ws.streak_days = 1;
        }
        ws.last_checkin_day = day;
        ws.total_checkins = ws.total_checkins.saturating_add(1);
        if ws.streak_days > ws.best_streak {
            ws.best_streak = ws.streak_days;
        }

        ctx.accounts.check_in.tokens = bal;
        let ds = &mut ctx.accounts.day_stats;
        if ds.checkins == 0 {
            ds.coin = coin_key;
            ds.day = day;
        }
        ds.checked_tokens = ds.checked_tokens.checked_add(bal).ok_or(EmberError::Math)?;
        ds.checkins = ds.checkins.checked_add(1).ok_or(EmberError::Math)?;
        Ok(())
    }

    /// Anyone, in order, once a day has ended. The community streak grows if
    /// wallets holding at least 30% of circulating supply checked in that day;
    /// otherwise it resets to zero.
    pub fn settle_day(ctx: Context<SettleDay>, day: u32) -> Result<()> {
        let now = Clock::get()?.unix_timestamp;
        let circulating = ctx.accounts.mint.supply.saturating_sub(ctx.accounts.token_vault.amount);
        let coin_key = ctx.accounts.coin.key();
        let coin = &mut ctx.accounts.coin;
        let ds = &mut ctx.accounts.day_stats;
        require!(coin.status != CoinStatus::Dead, EmberError::Dead);
        require!(day == coin.next_settle_day, EmberError::WrongSettleDay);
        require!(((day as i64) + 1) * DAY_SECS <= now, EmberError::DayNotEnded);
        require!((day as i64) * DAY_SECS < coin.end_ts, EmberError::DayAfterEnd);

        let passed = circulating > 0
            && (ds.checked_tokens as u128) * BPS >= (circulating as u128) * COMMUNITY_THRESHOLD_BPS;
        if passed {
            coin.community_streak = coin.community_streak.saturating_add(1);
            if coin.community_streak > coin.best_community_streak {
                coin.best_community_streak = coin.community_streak;
            }
        } else {
            coin.community_streak = 0;
        }
        ds.coin = coin_key;
        ds.day = day;
        ds.settled = true;
        ds.passed = passed;
        coin.next_settle_day = day.checked_add(1).ok_or(EmberError::Math)?;
        Ok(())
    }

    /// Claimable any time while the coin is alive (Live or Voting).
    pub fn claim_creator_fees(ctx: Context<ClaimCreatorFees>) -> Result<()> {
        let coin = &mut ctx.accounts.coin;
        require!(coin.status != CoinStatus::Dead, EmberError::Dead);
        let amount = coin.creator_fees_claimable;
        require!(amount > 0, EmberError::NothingToClaim);
        coin.creator_fees_claimable = 0;

        let coin_key = ctx.accounts.coin.key();
        let seeds: &[&[u8]] = &[b"creator_vault", coin_key.as_ref(), &[ctx.bumps.creator_vault]];
        transfer(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.creator_vault.to_account_info(),
                    to: ctx.accounts.creator.to_account_info(),
                },
                &[seeds],
            ),
            amount,
        )
    }
}

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(init, payer = admin, space = 8 + Config::INIT_SPACE, seeds = [b"config"], bump)]
    pub config: Account<'info, Config>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CreateCoin<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(init, payer = creator, space = 8 + Coin::INIT_SPACE,
        seeds = [b"coin", mint.key().as_ref()], bump)]
    pub coin: Account<'info, Coin>,
    #[account(init, payer = creator, mint::decimals = DECIMALS, mint::authority = coin,
        mint::token_program = token_program)]
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(init, payer = creator, associated_token::mint = mint,
        associated_token::authority = coin, associated_token::token_program = token_program)]
    pub token_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, seeds = [b"sol_vault", coin.key().as_ref()], bump)]
    pub sol_vault: SystemAccount<'info>,
    #[account(mut, seeds = [b"treasury", coin.key().as_ref()], bump)]
    pub treasury: SystemAccount<'info>,
    #[account(mut, seeds = [b"creator_vault", coin.key().as_ref()], bump)]
    pub creator_vault: SystemAccount<'info>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Trade<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [b"config"], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(mut, seeds = [b"coin", mint.key().as_ref()], bump = coin.bump, has_one = mint)]
    pub coin: Account<'info, Coin>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = coin,
        associated_token::token_program = token_program)]
    pub token_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(init_if_needed, payer = user, associated_token::mint = mint,
        associated_token::authority = user, associated_token::token_program = token_program)]
    pub user_token: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, seeds = [b"sol_vault", coin.key().as_ref()], bump)]
    pub sol_vault: SystemAccount<'info>,
    #[account(mut, seeds = [b"treasury", coin.key().as_ref()], bump)]
    pub treasury: SystemAccount<'info>,
    #[account(mut, seeds = [b"creator_vault", coin.key().as_ref()], bump)]
    pub creator_vault: SystemAccount<'info>,
    /// CHECK: must equal the wallet stored in config.
    #[account(mut, address = config.platform_wallet)]
    pub platform_wallet: UncheckedAccount<'info>,
    #[account(init_if_needed, payer = user, space = 8 + WalletState::INIT_SPACE,
        seeds = [b"wallet", coin.key().as_ref(), user.key().as_ref()], bump)]
    pub wallet_state: Account<'info, WalletState>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimCreatorFees<'info> {
    #[account(mut, address = coin.creator @ EmberError::NotCreator)]
    pub creator: Signer<'info>,
    #[account(mut)]
    pub coin: Account<'info, Coin>,
    #[account(mut, seeds = [b"creator_vault", coin.key().as_ref()], bump)]
    pub creator_vault: SystemAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CloseCoin<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(mut, seeds = [b"coin", mint.key().as_ref()], bump = coin.bump, has_one = mint)]
    pub coin: Account<'info, Coin>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(associated_token::mint = mint, associated_token::authority = coin,
        associated_token::token_program = token_program)]
    pub token_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(init, payer = caller, space = 8 + Vote::INIT_SPACE,
        seeds = [b"vote", coin.key().as_ref(), &[coin.life_index]], bump)]
    pub vote: Account<'info, Vote>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CastVote<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,
    #[account(seeds = [b"coin", mint.key().as_ref()], bump = coin.bump, has_one = mint)]
    pub coin: Account<'info, Coin>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut, seeds = [b"vote", coin.key().as_ref(), &[coin.life_index]], bump = vote.bump)]
    pub vote: Account<'info, Vote>,
    #[account(init_if_needed, payer = voter, space = 8 + VoteRecord::INIT_SPACE,
        seeds = [b"voted", vote.key().as_ref(), voter.key().as_ref()], bump)]
    pub record: Account<'info, VoteRecord>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = voter,
        associated_token::token_program = token_program)]
    pub voter_token: InterfaceAccount<'info, TokenAccount>,
    #[account(init_if_needed, payer = voter, associated_token::mint = mint,
        associated_token::authority = vote, associated_token::token_program = token_program)]
    pub escrow: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawVote<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,
    #[account(has_one = mint)]
    pub coin: Account<'info, Coin>,
    pub mint: InterfaceAccount<'info, Mint>,
    /// May belong to an earlier life, so it is not derived from the current life.
    #[account(mut, has_one = coin)]
    pub vote: Account<'info, Vote>,
    #[account(mut, seeds = [b"voted", vote.key().as_ref(), voter.key().as_ref()], bump)]
    pub record: Account<'info, VoteRecord>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = voter,
        associated_token::token_program = token_program)]
    pub voter_token: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = vote,
        associated_token::token_program = token_program)]
    pub escrow: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

#[derive(Accounts)]
pub struct FinalizeVote<'info> {
    pub caller: Signer<'info>,
    #[account(mut)]
    pub coin: Account<'info, Coin>,
    #[account(mut, has_one = coin)]
    pub vote: Account<'info, Vote>,
}

#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(mut, seeds = [b"coin", mint.key().as_ref()], bump = coin.bump, has_one = mint)]
    pub coin: Account<'info, Coin>,
    #[account(mut)]
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = coin,
        associated_token::token_program = token_program)]
    pub token_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = user,
        associated_token::token_program = token_program)]
    pub user_token: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, seeds = [b"sol_vault", coin.key().as_ref()], bump)]
    pub sol_vault: SystemAccount<'info>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(day: u32)]
pub struct CheckInCtx<'info> {
    #[account(mut)]
    pub user: Signer<'info>,
    #[account(seeds = [b"coin", mint.key().as_ref()], bump = coin.bump, has_one = mint)]
    pub coin: Account<'info, Coin>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(associated_token::mint = mint, associated_token::authority = user,
        associated_token::token_program = token_program)]
    pub user_token: InterfaceAccount<'info, TokenAccount>,
    #[account(init_if_needed, payer = user, space = 8 + WalletState::INIT_SPACE,
        seeds = [b"wallet", coin.key().as_ref(), user.key().as_ref()], bump)]
    pub wallet_state: Account<'info, WalletState>,
    #[account(init, payer = user, space = 8 + CheckInRecord::INIT_SPACE,
        seeds = [b"checkin", coin.key().as_ref(), &[coin.life_index], &day.to_le_bytes(), user.key().as_ref()],
        bump)]
    pub check_in: Account<'info, CheckInRecord>,
    #[account(init_if_needed, payer = user, space = 8 + DayStats::INIT_SPACE,
        seeds = [b"day", coin.key().as_ref(), &[coin.life_index], &day.to_le_bytes()], bump)]
    pub day_stats: Account<'info, DayStats>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(day: u32)]
pub struct SettleDay<'info> {
    #[account(mut)]
    pub caller: Signer<'info>,
    #[account(mut, seeds = [b"coin", mint.key().as_ref()], bump = coin.bump, has_one = mint)]
    pub coin: Account<'info, Coin>,
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(associated_token::mint = mint, associated_token::authority = coin,
        associated_token::token_program = token_program)]
    pub token_vault: InterfaceAccount<'info, TokenAccount>,
    #[account(init_if_needed, payer = caller, space = 8 + DayStats::INIT_SPACE,
        seeds = [b"day", coin.key().as_ref(), &[coin.life_index], &day.to_le_bytes()], bump)]
    pub day_stats: Account<'info, DayStats>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}
