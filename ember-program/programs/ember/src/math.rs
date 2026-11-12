//! Pure fee and curve math. No Solana types, so it is easy to unit test.

pub const BPS: u128 = 10_000;
pub const PLATFORM_FEE_BPS: u64 = 100; // fixed 1%, inside the total
pub const MAX_TOTAL_FEE_BPS: u64 = 500; // 5% cap including platform
pub const MIN_TREASURY_SHARE_BPS: u64 = 2_500; // of the creator fee
pub const MIN_QUORUM_BPS: u64 = 2_000;
pub const MAX_QUORUM_BPS: u64 = 5_000;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct FeeSplit {
    pub platform: u64,
    pub treasury: u64,
    pub creator: u64,
}

impl FeeSplit {
    pub fn total(&self) -> u64 {
        self.platform + self.treasury + self.creator
    }
}

/// Platform fee first, then the creator fee, of which `treasury_share_bps`
/// goes to the treasury (rounded up so it never falls below its minimum).
pub fn split_fee(amount: u64, total_fee_bps: u64, treasury_share_bps: u64) -> Option<FeeSplit> {
    if !(PLATFORM_FEE_BPS..=MAX_TOTAL_FEE_BPS).contains(&total_fee_bps) {
        return None;
    }
    if !(MIN_TREASURY_SHARE_BPS..=10_000).contains(&treasury_share_bps) {
        return None;
    }
    let a = amount as u128;
    let platform = a * PLATFORM_FEE_BPS as u128 / BPS;
    let creator_fee = a * (total_fee_bps - PLATFORM_FEE_BPS) as u128 / BPS;
    let treasury = (creator_fee * treasury_share_bps as u128 + BPS - 1) / BPS;
    let creator = creator_fee - treasury;
    Some(FeeSplit {
        platform: platform as u64,
        treasury: treasury as u64,
        creator: creator as u64,
    })
}

/// Constant product. Rounds in the pool's favour.
pub fn tokens_out(virtual_sol: u64, virtual_token: u64, sol_in: u64) -> Option<u64> {
    let k = virtual_sol as u128 * virtual_token as u128;
    let new_vs = virtual_sol as u128 + sol_in as u128;
    let new_vt = (k + new_vs - 1) / new_vs;
    u64::try_from((virtual_token as u128).checked_sub(new_vt)?).ok()
}

pub fn sol_out(virtual_sol: u64, virtual_token: u64, tokens_in: u64) -> Option<u64> {
    let k = virtual_sol as u128 * virtual_token as u128;
    let new_vt = virtual_token as u128 + tokens_in as u128;
    let new_vs = (k + new_vt - 1) / new_vt;
    u64::try_from((virtual_sol as u128).checked_sub(new_vs)?).ok()
}


pub const MAX_LIVES: u8 = 3; // the first life plus two revivals

/// Turnout: locked tokens as a share of circulating supply at close.
pub fn quorum_met(locked: u64, circulating: u64, quorum_bps: u16) -> bool {
    circulating > 0 && (locked as u128) * BPS >= (circulating as u128) * quorum_bps as u128
}

pub fn simple_majority(yes: u64, no: u64) -> bool {
    yes > no
}

pub fn two_thirds(yes: u64, no: u64) -> bool {
    let total = yes as u128 + no as u128;
    total > 0 && (yes as u128) * 3 >= total * 2
}

/// Pro rata share of the pool for `amount` of the circulating supply.
pub fn redeem_payout(real_sol: u64, amount: u64, circulating: u64) -> Option<u64> {
    if circulating == 0 || amount > circulating {
        return None;
    }
    u64::try_from(real_sol as u128 * amount as u128 / circulating as u128).ok()
}

/// After paying out SOL, shrink the virtual token reserve by the same ratio
/// as the virtual SOL reserve so the spot price stays continuous.
pub fn scaled_virtual_token(vt: u64, vs_after: u64, vs_before: u64) -> Option<u64> {
    if vs_before == 0 {
        return None;
    }
    u64::try_from(vt as u128 * vs_after as u128 / vs_before as u128).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fee_split_adds_up_and_respects_minimums() {
        let s = split_fee(1_000_000_000, 300, 2_500).unwrap();
        assert_eq!(s.platform, 10_000_000); // 1%
        assert_eq!(s.treasury, 5_000_000); // 25% of the 2% creator fee
        assert_eq!(s.creator, 15_000_000);
        assert_eq!(s.total(), 30_000_000);
    }

    #[test]
    fn rejects_bad_params() {
        assert!(split_fee(1000, 99, 2_500).is_none());
        assert!(split_fee(1000, 501, 2_500).is_none());
        assert!(split_fee(1000, 300, 2_499).is_none());
        assert!(split_fee(1000, 300, 10_001).is_none());
    }

    #[test]
    fn round_trip_never_pays_out_more_than_paid_in() {
        let (vs, vt) = (30_000_000_000u64, 1_000_000_000_000_000u64);
        let t = tokens_out(vs, vt, 1_000_000_000).unwrap();
        let back = sol_out(vs + 1_000_000_000, vt - t, t).unwrap();
        assert!(back <= 1_000_000_000);
    }

    #[test]
    fn quorum_and_majorities() {
        assert!(quorum_met(200, 1000, 2000));
        assert!(!quorum_met(199, 1000, 2000));
        assert!(!quorum_met(10, 0, 2000));
        assert!(simple_majority(6, 5));
        assert!(!simple_majority(5, 5));
        assert!(two_thirds(2, 1));
        assert!(!two_thirds(199, 101));
        assert!(!two_thirds(0, 0));
    }

    #[test]
    fn redeem_is_pro_rata_and_keeps_price() {
        let (real, circ) = (485_000_000u64, 16_000_000_000u64);
        let p = redeem_payout(real, circ / 4, circ).unwrap();
        assert_eq!(p, real / 4);
        assert!(redeem_payout(real, circ + 1, circ).is_none());
        let (vs, vt) = (30_000_000_000u64 + real, 900_000_000_000_000u64);
        let vt2 = scaled_virtual_token(vt, vs - p, vs).unwrap();
        // price vs/vt must not rise (rounding down in vt would raise it by < 1 unit)
        let lhs = (vs - p) as u128 * vt as u128;
        let rhs = vs as u128 * vt2 as u128;
        assert!(rhs <= lhs && lhs - rhs < vs as u128);
    }
}
