import * as anchor from "@coral-xyz/anchor";
import { BN, Program } from "@coral-xyz/anchor";
import { Keypair, PublicKey, SystemProgram, LAMPORTS_PER_SOL, SYSVAR_CLOCK_PUBKEY } from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID,
  getAssociatedTokenAddressSync, getAccount,
} from "@solana/spl-token";
import { assert } from "chai";

const provider = anchor.AnchorProvider.env();
anchor.setProvider(provider);
const program = anchor.workspace.Ember as Program<any>;
const pid = program.programId;
const user = provider.wallet.publicKey;
const pda = (seeds: (Buffer | Uint8Array)[]) => PublicKey.findProgramAddressSync(seeds, pid)[0];
const ata = (mint: PublicKey, owner: PublicKey, off = false) =>
  getAssociatedTokenAddressSync(mint, owner, off, TOKEN_2022_PROGRAM_ID);
const SOL = (n: number) => new BN(Math.round(n * LAMPORTS_PER_SOL));
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const bal = (k: PublicKey) => provider.connection.getBalance(k);
const tokBal = async (k: PublicKey) =>
  (await getAccount(provider.connection, k, "processed", TOKEN_2022_PROGRAM_ID)).amount;

async function fails(p: Promise<any>, text: string) {
  try { await p; } catch (e: any) { assert.include(String(e), text); return; }
  assert.fail(`expected failure containing: ${text}`);
}

const platform = Keypair.generate();
const config = pda([Buffer.from("config")]);
const sys = SystemProgram.programId;

function ids(mint: PublicKey) {
  const coin = pda([Buffer.from("coin"), mint.toBuffer()]);
  return {
    coin, mint,
    tokenVault: ata(mint, coin, true),
    solVault: pda([Buffer.from("sol_vault"), coin.toBuffer()]),
    treasury: pda([Buffer.from("treasury"), coin.toBuffer()]),
    creatorVault: pda([Buffer.from("creator_vault"), coin.toBuffer()]),
    userToken: ata(mint, user),
    walletState: pda([Buffer.from("wallet"), coin.toBuffer(), user.toBuffer()]),
  };
}
const createAccounts = (m: PublicKey) => {
  const i = ids(m);
  return {
    creator: user, coin: i.coin, mint: m, tokenVault: i.tokenVault, solVault: i.solVault,
    treasury: i.treasury, creatorVault: i.creatorVault, tokenProgram: TOKEN_2022_PROGRAM_ID,
    associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID, systemProgram: sys,
  };
};
const tradeAccounts = (m: PublicKey) => {
  const i = ids(m);
  return {
    user, config, coin: i.coin, mint: m, tokenVault: i.tokenVault, userToken: i.userToken,
    solVault: i.solVault, treasury: i.treasury, creatorVault: i.creatorVault,
    platformWallet: platform.publicKey, walletState: i.walletState,
    tokenProgram: TOKEN_2022_PROGRAM_ID, associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    systemProgram: sys,
  };
};
const votePda = (coin: PublicKey, life: number) =>
  pda([Buffer.from("vote"), coin.toBuffer(), Buffer.from([life])]);
const recordPda = (vote: PublicKey) =>
  pda([Buffer.from("voted"), vote.toBuffer(), user.toBuffer()]);

describe("ember", () => {
  const mint = Keypair.generate();
  const A = ids(mint.publicKey);
  const create = (m: Keypair, life: number, fee: number, share: number, quorum: number, mode: number) =>
    program.methods.createCoin(new BN(life), fee, share, quorum, mode)
      .accounts(createAccounts(m.publicKey)).signers([m]).rpc();

  it("initializes config", async () => {
    await program.methods.initializeConfig(platform.publicKey)
      .accounts({ admin: user, config, publicPool: pda([Buffer.from("pool")]), systemProgram: sys }).rpc();
    const c = await program.account.config.fetch(config);
    assert.equal(c.platformWallet.toBase58(), platform.publicKey.toBase58());
  });

  it("rejects bad launch parameters", async () => {
    const cases: [number, number, number, number, number, string][] = [
      [1000, 300, 5000, 2000, 0, "BadLifespan"],
      [604800, 600, 5000, 2000, 0, "BadFee"],
      [604800, 50, 5000, 2000, 0, "BadFee"],
      [604800, 300, 2000, 2000, 0, "BadTreasuryShare"],
      [604800, 300, 5000, 1500, 0, "BadQuorum"],
      [604800, 300, 5000, 6000, 0, "BadQuorum"],
      [604800, 300, 5000, 2000, 2, "BadVoteMode"],
    ];
    for (const [life, fee, share, quorum, mode, err] of cases) {
      await fails(create(Keypair.generate(), life, fee, share, quorum, mode), err);
    }
  });

  it("creates a coin with locked settings and full supply in the vault", async () => {
    await create(mint, 604800, 300, 5000, 2000, 0);
    const c = await program.account.coin.fetch(A.coin);
    assert.equal(c.totalFeeBps, 300);
    assert.equal(c.treasuryShareBps, 5000);
    assert.equal(c.quorumBps, 2000);
    assert.equal(c.lifeIndex, 1);
    assert.equal((await tokBal(A.tokenVault)).toString(), "1000000000000000");
  });

  it("buy splits fees exactly: 1% platform, 1% treasury, 1% creator", async () => {
    const [p0, t0, c0] = await Promise.all([bal(platform.publicKey), bal(A.treasury), bal(A.creatorVault)]);
    await program.methods.buy(SOL(0.5), new BN(1)).accounts(tradeAccounts(mint.publicKey)).rpc();
    assert.equal((await bal(platform.publicKey)) - p0, 5_000_000);
    assert.equal((await bal(A.treasury)) - t0, 5_000_000);
    assert.equal((await bal(A.creatorVault)) - c0, 5_000_000);
    const c = await program.account.coin.fetch(A.coin);
    assert.equal(c.realSol.toString(), "485000000");
    assert.equal(c.creatorFeesClaimable.toString(), "5000000");
    assert.isTrue((await tokBal(A.userToken)) > 0n);
  });

  it("no wallet cap: a second 0.5 SOL buy works", async () => {
    await program.methods.buy(SOL(0.5), new BN(1)).accounts(tradeAccounts(mint.publicKey)).rpc();
  });

  it("enforces slippage on buy", async () => {
    await fails(
      program.methods.buy(SOL(0.01), new BN("1000000000000000")).accounts(tradeAccounts(mint.publicKey)).rpc(),
      "Slippage");
  });

  it("creator claims fees any time while alive, once; only the creator", async () => {
    const acc = { creator: user, coin: A.coin, creatorVault: A.creatorVault, systemProgram: sys };
    const claimable = (await program.account.coin.fetch(A.coin)).creatorFeesClaimable.toNumber();
    const before = await bal(A.creatorVault);
    await program.methods.claimCreatorFees().accounts(acc).rpc();
    assert.equal(before - (await bal(A.creatorVault)), claimable);
    await fails(program.methods.claimCreatorFees().accounts(acc).rpc(), "NothingToClaim");
    const other = Keypair.generate();
    await provider.connection.requestAirdrop(other.publicKey, LAMPORTS_PER_SOL);
    await fails(
      program.methods.claimCreatorFees().accounts({ ...acc, creator: other.publicKey }).signers([other]).rpc(),
      "NotCreator");
  });

  it("sell returns SOL minus fees and never drains more than the pool holds", async () => {
    const half = new BN(((await tokBal(A.userToken)) / 2n).toString());
    const before = await program.account.coin.fetch(A.coin);
    await program.methods.sell(half, new BN(1)).accounts(tradeAccounts(mint.publicKey)).rpc();
    const after = await program.account.coin.fetch(A.coin);
    assert.isTrue(after.realSol.lt(before.realSol));
    assert.isTrue(after.realSol.gtn(0));
    assert.isTrue((await bal(A.solVault)) >= after.realSol.toNumber());
  });

  it("cannot close or redeem a live coin", async () => {
    await fails(
      program.methods.redeem(new BN(1)).accounts({
        user, coin: A.coin, mint: A.mint, tokenVault: A.tokenVault, userToken: A.userToken,
        solVault: A.solVault, tokenProgram: TOKEN_2022_PROGRAM_ID, systemProgram: sys,
      }).rpc(), "CannotRedeem");
  });
});

// Needs a build with --features short-life (3 second lifespan, 8 second vote).
describe("ending: close, vote, withdraw, revive, die, redeem", function () {
  this.timeout(180000);
  const m = Keypair.generate();
  const I = ids(m.publicKey);

  const closeCoin = async () => {
    const life = (await program.account.coin.fetch(I.coin)).lifeIndex;
    const vote = votePda(I.coin, life);
    await program.methods.closeCoin().accounts({
      caller: user, coin: I.coin, mint: m.publicKey, tokenVault: I.tokenVault, vote,
      tokenProgram: TOKEN_2022_PROGRAM_ID, systemProgram: sys,
    }).rpc();
    return vote;
  };
  const voteAccounts = (vote: PublicKey) => ({
    voter: user, coin: I.coin, mint: m.publicKey, vote, record: recordPda(vote),
    voterToken: I.userToken, escrow: ata(m.publicKey, vote, true),
    tokenProgram: TOKEN_2022_PROGRAM_ID, associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
    systemProgram: sys,
  });
  const cast = (vote: PublicKey, amount: bigint, revive: boolean, release: boolean) =>
    program.methods.castVote(new BN(amount.toString()), revive, release).accounts(voteAccounts(vote)).rpc();
  const withdraw = (vote: PublicKey) => {
    const a = voteAccounts(vote);
    return program.methods.withdrawVote().accounts({
      voter: user, coin: I.coin, mint: m.publicKey, vote, record: a.record,
      voterToken: I.userToken, escrow: a.escrow, tokenProgram: TOKEN_2022_PROGRAM_ID,
    }).rpc();
  };
  const finalize = (vote: PublicKey) =>
    program.methods.finalizeVote().accounts({ caller: user, coin: I.coin, vote }).rpc();
  const statusOf = async () => Object.keys((await program.account.coin.fetch(I.coin)).status)[0];

  let vote1: PublicKey;

  before(async function () {
    try {
      await program.methods.createCoin(new BN(3), 300, 5000, 2000, 0)
        .accounts(createAccounts(m.publicKey)).signers([m]).rpc();
    } catch (e) {
      if (String(e).includes("BadLifespan")) this.skip();
      throw e;
    }
    await program.methods.buy(SOL(0.5), new BN(1)).accounts(tradeAccounts(m.publicKey)).rpc();
  });

  it("cannot close before the timer", async () => {
    await fails(closeCoin(), "NotEnded");
  });

  it("closes after the timer and stops trading", async () => {
    await sleep(5000);
    vote1 = await closeCoin();
    assert.equal(await statusOf(), "voting");
    await fails(
      program.methods.buy(SOL(0.1), new BN(1)).accounts(tradeAccounts(m.publicKey)).rpc(), "NotLive");
  });

  it("voting locks tokens; withdrawing cancels the ballot", async () => {
    const all = await tokBal(I.userToken);
    await cast(vote1, all, true, false);
    assert.equal((await tokBal(I.userToken)).toString(), "0");
    let v = await program.account.vote.fetch(vote1);
    assert.equal(v.lockedTokens.toString(), all.toString());
    assert.equal(v.reviveYesTokens.toString(), all.toString());
    await fails(cast(vote1, 1n, true, false), "AlreadyVoted");
    await withdraw(vote1);
    v = await program.account.vote.fetch(vote1);
    assert.equal(v.lockedTokens.toString(), "0");
    assert.equal(v.reviveYesTokens.toString(), "0");
    assert.equal((await tokBal(I.userToken)).toString(), all.toString());
  });

  it("revive passes with quorum and majority: same curve, new life", async () => {
    const all = await tokBal(I.userToken);
    await cast(vote1, all, true, false);
    await fails(finalize(vote1), "VoteStillOpen");
    await sleep(10000);
    await finalize(vote1);
    const c = await program.account.coin.fetch(I.coin);
    assert.equal(c.lifeIndex, 2);
    assert.equal(Object.keys(c.status)[0], "live");
    await withdraw(vote1); // after finalize: tokens simply return
    assert.equal((await tokBal(I.userToken)).toString(), all.toString());
    await program.methods.buy(SOL(0.1), new BN(1)).accounts(tradeAccounts(m.publicKey)).rpc();
  });

  it("with no votes the coin dies, and holders redeem pro rata", async () => {
    await sleep(5000);
    const vote2 = await closeCoin();
    await sleep(10000);
    await finalize(vote2);
    assert.equal(await statusOf(), "dead");
    const all = await tokBal(I.userToken);
    const before = await bal(user);
    await program.methods.redeem(new BN(all.toString())).accounts({
      user, coin: I.coin, mint: m.publicKey, tokenVault: I.tokenVault, userToken: I.userToken,
      solVault: I.solVault, tokenProgram: TOKEN_2022_PROGRAM_ID, systemProgram: sys,
    }).rpc();
    assert.isTrue((await bal(user)) > before);
    assert.equal((await program.account.coin.fetch(I.coin)).realSol.toString(), "0");
    assert.equal((await tokBal(I.userToken)).toString(), "0");
  });
});

// Needs a build with --features short-life (6 second days, 40 second life).
describe("check-in and community streak", function () {
  this.timeout(180000);
  const DAY = 6;
  const m = Keypair.generate();
  const I = ids(m.publicKey);
  // Use the validator's own clock: it can lag the wall clock by seconds.
  const chainNow = async () =>
    Number((await provider.connection.getAccountInfo(SYSVAR_CLOCK_PUBKEY, "processed"))!.data.readBigInt64LE(32));
  const curDay = async () => Math.floor((await chainNow()) / DAY);
  const logErr = (e: any) => { console.log("LOGS:", (e.logs ?? []).slice(-8)); throw e; };
  const dayBuf = (d: number) => { const b = Buffer.alloc(4); b.writeUInt32LE(d); return b; };
  const dayStats = (d: number) =>
    pda([Buffer.from("day"), I.coin.toBuffer(), Buffer.from([1]), dayBuf(d)]);
  const checkIn = (d: number) =>
    program.methods.checkIn(d).accounts({
      user, coin: I.coin, mint: m.publicKey, userToken: I.userToken, walletState: I.walletState,
      weight: pda([Buffer.from("weight"), I.coin.toBuffer(), Buffer.from([0]), user.toBuffer()]),
      checkIn: pda([Buffer.from("checkin"), I.coin.toBuffer(), Buffer.from([1]), dayBuf(d), user.toBuffer()]),
      dayStats: dayStats(d), tokenProgram: TOKEN_2022_PROGRAM_ID, systemProgram: sys,
    }).rpc().catch(logErr);
  const settle = (d: number) =>
    program.methods.settleDay(d).accounts({
      caller: user, coin: I.coin, mint: m.publicKey, tokenVault: I.tokenVault,
      dayStats: dayStats(d), tokenProgram: TOKEN_2022_PROGRAM_ID, systemProgram: sys,
    }).rpc().catch(logErr);
  const waitForDay = async (d: number) => { while ((await curDay()) < d) await sleep(200); await sleep(400); };
  let startDay = 0;

  before(async function () {
    // start just after a day boundary so the first few calls land in one day
    while (((await chainNow()) % DAY) > 1) await sleep(100);
    try {
      await program.methods.createCoin(new BN(40), 300, 5000, 2000, 0)
        .accounts(createAccounts(m.publicKey)).signers([m]).rpc();
    } catch (e) {
      if (String(e).includes("BadLifespan")) this.skip();
      throw e;
    }
    await program.methods.buy(SOL(0.5), new BN(1)).accounts(tradeAccounts(m.publicKey)).rpc();
    startDay = (await program.account.coin.fetch(I.coin)).nextSettleDay;
  });

  it("checks in once per day and starts a streak", async () => {
    await checkIn(startDay);
    const ws = await program.account.walletState.fetch(I.walletState);
    assert.equal(ws.streakDays, 1);
    assert.equal(ws.totalCheckins, 1);
    await fails(checkIn(startDay), "already in use");
  });

  it("rejects a check-in for the wrong day", async () => {
    await fails(checkIn(startDay + 5), "WrongDay");
  });

  it("checking in the next day continues the streak", async () => {
    await waitForDay(startDay + 1);
    await checkIn(startDay + 1);
    const ws = await program.account.walletState.fetch(I.walletState);
    assert.equal(ws.streakDays, 2);
    assert.equal(ws.bestStreak, 2);
  });

  it("cannot settle a day that has not ended", async () => {
    await fails(settle(startDay + 1), "WrongSettleDay");
    await waitForDay(startDay + 2);
    await settle(startDay);
    await fails(settle(startDay + 2), "WrongSettleDay");
  });

  it("settles days in order and grows the community streak", async () => {
    let c = await program.account.coin.fetch(I.coin);
    assert.equal(c.communityStreak, 1);
    assert.equal(c.nextSettleDay, startDay + 1);
    await settle(startDay + 1);
    c = await program.account.coin.fetch(I.coin);
    assert.equal(c.communityStreak, 2);
    assert.equal(c.bestCommunityStreak, 2);
    await fails(settle(startDay + 2), "DayNotEnded");
  });

  it("a day with no check-ins resets the community streak", async () => {
    await waitForDay(startDay + 3);
    await settle(startDay + 2); // nobody checked in on this day
    const c = await program.account.coin.fetch(I.coin);
    assert.equal(c.communityStreak, 0);
    assert.equal(c.bestCommunityStreak, 2);
  });
});

// Needs a build with --features short-life (12 second life, 8 second vote, 5 second claim window).
describe("treasury payout", function () {
  this.timeout(240000);
  const m = Keypair.generate();
  const I = ids(m.publicKey);
  const pool = pda([Buffer.from("pool")]);
  const clockNow = async () =>
    Number((await provider.connection.getAccountInfo(SYSVAR_CLOCK_PUBKEY, "processed"))!.data.readBigInt64LE(32));
  const dayBuf = (d: number) => { const b = Buffer.alloc(4); b.writeUInt32LE(d); return b; };
  const distPda = (e: number) => pda([Buffer.from("dist"), I.coin.toBuffer(), Buffer.from([e])]);
  const weightPda = (e: number) =>
    pda([Buffer.from("weight"), I.coin.toBuffer(), Buffer.from([e]), user.toBuffer()]);
  let vote: PublicKey;
  let cap = 0;
  let snapshot = 0;

  const claimAccounts = () => ({
    user, config, coin: I.coin, distribution: distPda(0), weight: weightPda(0),
    treasury: I.treasury, systemProgram: sys,
  });

  before(async function () {
    try {
      await program.methods.createCoin(new BN(12), 300, 5000, 2000, 0)
        .accounts(createAccounts(m.publicKey)).signers([m]).rpc();
    } catch (e) {
      if (String(e).includes("BadLifespan")) this.skip();
      throw e;
    }
    await program.methods.buy(SOL(0.5), new BN(1)).accounts(tradeAccounts(m.publicKey)).rpc();
    const day = Math.floor((await clockNow()) / 6);
    await program.methods.checkIn(day).accounts({
      user, coin: I.coin, mint: m.publicKey, userToken: I.userToken, walletState: I.walletState,
      weight: weightPda(0),
      checkIn: pda([Buffer.from("checkin"), I.coin.toBuffer(), Buffer.from([1]), dayBuf(day), user.toBuffer()]),
      dayStats: pda([Buffer.from("day"), I.coin.toBuffer(), Buffer.from([1]), dayBuf(day)]),
      tokenProgram: TOKEN_2022_PROGRAM_ID, systemProgram: sys,
    }).rpc();
  });

  it("check-in adds reward weight: tokens held x streak", async () => {
    const w = await program.account.weightRecord.fetch(weightPda(0));
    const bal = await tokBal(I.userToken);
    assert.equal(w.weight.toString(), bal.toString()); // streak is 1 on the first day
    const c = await program.account.coin.fetch(I.coin);
    assert.equal(c.epochWeight.toString(), bal.toString());
  });

  it("the coin ends; holders vote to release the treasury and the coin dies", async () => {
    const end = (await program.account.coin.fetch(I.coin)).endTs.toNumber();
    while ((await clockNow()) < end + 1) await sleep(300);
    vote = votePda(I.coin, 1);
    await program.methods.closeCoin().accounts({
      caller: user, coin: I.coin, mint: m.publicKey, tokenVault: I.tokenVault, vote,
      tokenProgram: TOKEN_2022_PROGRAM_ID, systemProgram: sys,
    }).rpc();
    const all = await tokBal(I.userToken);
    await program.methods.castVote(new BN(all.toString()), false, true).accounts({
      voter: user, coin: I.coin, mint: m.publicKey, vote, record: recordPda(vote),
      voterToken: I.userToken, escrow: ata(m.publicKey, vote, true),
      tokenProgram: TOKEN_2022_PROGRAM_ID, associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: sys,
    }).rpc();
    await sleep(10000);
    await program.methods.finalizeVote().accounts({ caller: user, coin: I.coin, vote }).rpc();
    const c = await program.account.coin.fetch(I.coin);
    assert.equal(Object.keys(c.status)[0], "dead");
    assert.isTrue((await program.account.vote.fetch(vote)).releasePassed);
  });

  it("opens a distribution that snapshots the treasury and the weights", async () => {
    await program.methods.openDistribution().accounts({
      caller: user, coin: I.coin, vote, treasury: I.treasury, distribution: distPda(0), systemProgram: sys,
    }).rpc();
    const d = await program.account.distribution.fetch(distPda(0));
    snapshot = d.snapshot.toNumber();
    assert.isTrue(snapshot > 0);
    assert.isTrue(d.totalWeight.gtn(0));
    const c = await program.account.coin.fetch(I.coin);
    assert.equal(c.epoch, 1);
    assert.equal(c.treasuryReserved.toNumber(), snapshot);
    // the same vote cannot open a second distribution
    await fails(
      program.methods.openDistribution().accounts({
        caller: user, coin: I.coin, vote, treasury: I.treasury, distribution: distPda(1), systemProgram: sys,
      }).rpc(), "NotEligible");
  });

  it("claims are blocked until payouts are enabled", async () => {
    await fails(program.methods.claimReward(0).accounts(claimAccounts()).rpc(), "PayoutsDisabled");
  });

  it("pays pro rata, capped at 5% of the snapshot, and only once", async () => {
    await program.methods.setPayoutsEnabled(true).accounts({ admin: user, config }).rpc();
    cap = Math.floor((snapshot * 500) / 10000);
    const before = await bal(I.treasury);
    await program.methods.claimReward(0).accounts(claimAccounts()).rpc();
    assert.equal(before - (await bal(I.treasury)), cap);
    await fails(program.methods.claimReward(0).accounts(claimAccounts()).rpc(), "AlreadyClaimed");
    const d = await program.account.distribution.fetch(distPda(0));
    assert.equal(d.claimedTotal.toNumber(), cap);
  });

  it("sweeps the leftover to the public pool after the claim window, never to the creator", async () => {
    const sweepAccounts = {
      caller: user, config, coin: I.coin, distribution: distPda(0), treasury: I.treasury,
      publicPool: pool, systemProgram: sys,
    };
    await fails(program.methods.sweepUnclaimed(0).accounts(sweepAccounts).rpc(), "ClaimWindowOpen");
    await sleep(6500);
    const before = await bal(pool);
    const creatorBefore = await bal(I.creatorVault);
    await program.methods.sweepUnclaimed(0).accounts(sweepAccounts).rpc();
    assert.equal((await bal(pool)) - before, snapshot - cap);
    assert.equal(await bal(I.creatorVault), creatorBefore);
    assert.equal((await program.account.coin.fetch(I.coin)).treasuryReserved.toNumber(), 0);
    await fails(program.methods.sweepUnclaimed(0).accounts(sweepAccounts).rpc(), "AlreadySwept");
  });
});
