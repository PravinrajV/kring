//! Unit tests: one block per attack in THREAT_MODEL.md, plus the money invariants.

use super::*;
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{testing_env, PromiseResult, RuntimeFeesConfig};

const PERIOD: u64 = 120; // seconds
const UNIT: u128 = 250_000_000; // 250 tUSD at 6 decimals
const SEC: u64 = 1_000_000_000;
const T0: u64 = 1_700_000_000 * SEC;

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}
fn token() -> AccountId {
    acc("tusd.test.near")
}
fn circle_id() -> AccountId {
    acc("circle.test.near")
}
fn m(i: usize) -> AccountId {
    acc(&format!("m{:02}.test.near", i))
}

fn cfg() -> Config {
    Config {
        period_secs: PERIOD as u32,
        waiting_periods: 1,
        probation_periods: 2,
        max_gift_periods: 24,
        vouch_window_secs: 60,
        vouch_cooldown_periods: 12,
        proposal_secs: 300,
        gift_unit: U128(UNIT),
    }
}

/// Sets the caller and the clock. `secs` is seconds after genesis.
fn at(secs: u64, who: &AccountId) {
    at_seed(secs, who, [7u8; 32]);
}
fn at_seed(secs: u64, who: &AccountId, seed: [u8; 32]) {
    testing_env!(VMContextBuilder::new()
        .current_account_id(circle_id())
        .predecessor_account_id(who.clone())
        .block_timestamp(T0 + secs * SEC)
        .random_seed(seed)
        .attached_deposit(PROPOSAL_BOND)
        .build());
}
fn upgrade_ctx(who: AccountId, input: Vec<u8>) {
    let mut ctx = VMContextBuilder::new()
        .current_account_id(circle_id())
        .predecessor_account_id(who)
        .block_timestamp(T0 + 13 * SEC)
        .build();
    ctx.input = input.into();
    testing_env!(ctx);
}
fn period_start(p: u64) -> u64 {
    p * PERIOD
}

struct World {
    c: Circle,
    n: usize,
}

fn world(n: usize, level: u8) -> World {
    // Fresh storage for every circle, even when one test builds several.
    near_sdk::mock::with_mocked_blockchain(|b| {
        b.take_storage();
    });
    at(0, &circle_id());
    let founders = (0..n).map(|i| FounderInput { account_id: m(i), level }).collect();
    World { c: Circle::new(token(), founders, cfg()), n }
}

impl World {
    fn deposit(&mut self, secs: u64, who: &AccountId, amount: u128) -> u128 {
        at(secs, &token());
        match self.c.ft_on_transfer(who.clone(), U128(amount), String::new()) {
            PromiseOrValue::Value(v) => v.0,
            _ => panic!("expected value"),
        }
    }
    /// Every member deposits `periods` worth of their level at time `secs`.
    fn fund_all(&mut self, secs: u64, periods: u128) {
        for i in 0..self.n {
            let lvl = self.c.members.get(&m(i)).unwrap().level as u128;
            let amt = lvl * UNIT / 20 * periods;
            assert_eq!(self.deposit(secs, &m(i), amt), 0);
        }
    }
    fn settle_through(&mut self, secs: u64, who: &AccountId) -> Vec<Settlement> {
        let mut out = vec![];
        loop {
            at(secs, who);
            if self.c.get_circle().settle_ready {
                out.push(self.c.settle());
            } else {
                return out;
            }
        }
    }
    fn pot(&self, a: &AccountId) -> u128 {
        self.c.members.get(a).unwrap().pot
    }
    fn gifts(&self, a: &AccountId) -> u128 {
        self.c.members.get(a).unwrap().gift_balance
    }
    fn check_books(&self) {
        let pots: u128 = self.c.members.values().map(|m| m.pot).sum();
        let gifts: u128 = self.c.members.values().map(|m| m.gift_balance).sum();
        assert_eq!(pots, self.c.total_pots, "pot total drifted");
        assert_eq!(gifts, self.c.total_gift_balances, "gift total drifted");
        assert_eq!(self.c.get_held().0, pots + gifts);
    }
    /// Opens a report for `who` at `secs` and has both drawn vouchers vouch.
    fn sick_and_confirmed(&mut self, secs: u64, who: &AccountId) {
        at(secs, who);
        let v = self.c.report_sick();
        for x in v {
            at(secs + 1, &x);
            self.c.vouch(who.clone());
        }
    }
}

// ---------- setup and config ----------

#[test]
#[should_panic(expected = "Only the circle account can initialize itself")]
fn init_only_by_the_circle_account_itself() {
    at(0, &m(0));
    let founders = (0..3).map(|i| FounderInput { account_id: m(i), level: 1 }).collect();
    Circle::new(token(), founders, cfg());
}

#[test]
#[should_panic(expected = "A circle needs 3 to 50 founders")]
fn init_rejects_too_few_founders() {
    at(0, &circle_id());
    Circle::new(token(), vec![FounderInput { account_id: m(0), level: 1 }], cfg());
}

#[test]
#[should_panic(expected = "max_gift_periods must be 1 to 24")]
fn init_rejects_more_than_24_gift_periods() {
    at(0, &circle_id());
    let mut c = cfg();
    c.max_gift_periods = 25;
    let founders = (0..3).map(|i| FounderInput { account_id: m(i), level: 1 }).collect();
    Circle::new(token(), founders, c);
}

#[test]
#[should_panic(expected = "waiting_periods must be at least 1")]
fn init_rejects_no_waiting_period() {
    at(0, &circle_id());
    let mut c = cfg();
    c.waiting_periods = 0;
    let founders = (0..3).map(|i| FounderInput { account_id: m(i), level: 1 }).collect();
    Circle::new(token(), founders, c);
}

#[test]
#[should_panic(expected = "Duplicate founder")]
fn init_rejects_duplicate_founders() {
    at(0, &circle_id());
    let founders = vec![
        FounderInput { account_id: m(0), level: 1 },
        FounderInput { account_id: m(0), level: 1 },
        FounderInput { account_id: m(1), level: 1 },
    ];
    Circle::new(token(), founders, cfg());
}

// ---------- attack 1: someone keeps control ----------

#[test]
#[should_panic(expected = "Not a member")]
fn outsider_cannot_propose() {
    let mut w = world(5, 1);
    at(10, &acc("deployer.test.near"));
    w.c.propose(ProposalKind::Admit { account_id: acc("friend.test.near"), level: 1 });
}

#[test]
#[should_panic(expected = "Upgrades need two thirds of active members")]
fn upgrade_needs_two_thirds() {
    let mut w = world(6, 1);
    let code = b"\0asm fake new code".to_vec();
    let hash: String = env::sha256(&code).iter().map(|b| format!("{:02x}", b)).collect();
    at(10, &m(0));
    let id = w.c.propose(ProposalKind::Upgrade { code_hash: hash });
    at(11, &m(1));
    w.c.approve(id);
    at(12, &m(2));
    w.c.approve(id); // 3 of 6, not enough
    let mut input = id.to_le_bytes().to_vec();
    input.extend(code);
    upgrade_ctx(m(0), input);
    let _ = w.c.execute_upgrade();
}

#[test]
#[should_panic(expected = "Code doesn't match the approved hash")]
fn upgrade_rejects_different_code() {
    let mut w = world(3, 1);
    let hash: String = env::sha256(b"approved").iter().map(|b| format!("{:02x}", b)).collect();
    at(10, &m(0));
    let id = w.c.propose(ProposalKind::Upgrade { code_hash: hash });
    for i in 1..3 {
        at(11, &m(i));
        w.c.approve(id);
    }
    let mut input = id.to_le_bytes().to_vec();
    input.extend(b"something else".to_vec());
    upgrade_ctx(m(0), input);
    let _ = w.c.execute_upgrade();
}

#[test]
fn upgrade_passes_with_two_thirds_and_matching_code() {
    let mut w = world(6, 1);
    let code = b"\0asm new code".to_vec();
    let hash: String = env::sha256(&code).iter().map(|b| format!("{:02x}", b)).collect();
    at(10, &m(0));
    let id = w.c.propose(ProposalKind::Upgrade { code_hash: hash });
    for i in 1..4 {
        at(11, &m(i));
        w.c.approve(id);
    }
    let mut input = id.to_le_bytes().to_vec();
    input.extend(code);
    upgrade_ctx(acc("anyone.test.near"), input);
    let _ = w.c.execute_upgrade();
    assert_eq!(w.c.get_proposals(None, None)[0].status, ProposalStatus::Executed);
}

// ---------- attack 2: faking sickness ----------

#[test]
fn unvouched_report_gets_nothing() {
    let mut w = world(5, 1);
    w.fund_all(0, 10);
    at(period_start(3), &m(0));
    w.c.report_sick();
    let s = w.settle_through(period_start(6), &m(4));
    assert!(s.iter().all(|x| x.gifts.is_empty()));
    assert_eq!(w.gifts(&m(0)), 0);
    w.check_books();
}

#[test]
#[should_panic(expected = "You weren't drawn to vouch for this report")]
fn only_drawn_vouchers_can_vouch() {
    let mut w = world(10, 1);
    w.fund_all(0, 10);
    at(period_start(3), &m(0));
    let v = w.c.report_sick();
    let outsider = (1..10).map(m).find(|a| !v.contains(a)).unwrap();
    at(period_start(3) + 1, &outsider);
    w.c.vouch(m(0));
}

#[test]
#[should_panic(expected = "Not a member")]
fn non_member_cannot_vouch() {
    let mut w = world(5, 1);
    w.fund_all(0, 10);
    at(period_start(3), &m(0));
    w.c.report_sick();
    at(period_start(3) + 1, &acc("stranger.test.near"));
    w.c.vouch(m(0));
}

#[test]
#[should_panic(expected = "You already vouched")]
fn one_voucher_cannot_count_twice() {
    let mut w = world(5, 1);
    w.fund_all(0, 10);
    at(period_start(3), &m(0));
    let v = w.c.report_sick();
    at(period_start(3) + 1, &v[0]);
    w.c.vouch(m(0));
    w.c.vouch(m(0));
}

#[test]
fn expelling_a_fake_claimant_stops_gifts_at_once() {
    let mut w = world(6, 1);
    w.fund_all(0, 20);
    w.sick_and_confirmed(period_start(3), &m(0));
    w.settle_through(period_start(5), &m(5)); // periods 0-4 settled, gift for period 4 paid
    let before = w.gifts(&m(0));
    assert_eq!(before, UNIT);
    at(period_start(5) + 1, &m(1));
    let id = w.c.propose(ProposalKind::Expel { account_id: m(0) });
    for i in 2..4 {
        at(period_start(5) + 2, &m(i));
        w.c.approve(id);
    }
    at(period_start(5) + 3, &m(5));
    w.c.execute(id);
    let s = w.settle_through(period_start(8), &m(5));
    assert!(s.iter().all(|x| x.gifts.is_empty()));
    assert_eq!(w.gifts(&m(0)), before);
    assert!(w.c.get_member(m(0)).unwrap().expelled);
    w.check_books();
}

#[test]
#[should_panic(expected = "You can't vote on your own expulsion")]
fn cannot_vote_on_own_expulsion() {
    let mut w = world(5, 1);
    at(10, &m(1));
    let id = w.c.propose(ProposalKind::Expel { account_id: m(0) });
    at(11, &m(0));
    w.c.approve(id);
}

#[test]
#[should_panic(expected = "Needs approval from more than half of active members")]
fn expulsion_needs_a_majority() {
    let mut w = world(6, 1);
    at(10, &m(1));
    let id = w.c.propose(ProposalKind::Expel { account_id: m(0) });
    at(11, &m(2));
    w.c.approve(id); // 2 of 5 eligible voters
    at(12, &m(3));
    w.c.execute(id);
}

// ---------- attack 3: joining already sick ----------

#[test]
#[should_panic(expected = "Still in probation")]
fn new_member_cannot_report_during_probation() {
    let mut w = world(5, 1);
    w.fund_all(0, 5);
    at(10, &m(0));
    w.c.report_sick();
}

#[test]
#[should_panic(expected = "Still in probation")]
fn admitted_member_has_probation_too() {
    let mut w = world(5, 1);
    let newbie = acc("newbie.test.near");
    at(period_start(3), &m(0));
    let id = w.c.propose(ProposalKind::Admit { account_id: newbie.clone(), level: 1 });
    for i in 1..3 {
        at(period_start(3) + 1, &m(i));
        w.c.approve(id);
    }
    at(period_start(3) + 300, &m(4));
    w.c.execute(id);
    w.deposit(period_start(3) + 301, &newbie, UNIT);
    at(period_start(4), &newbie);
    w.c.report_sick();
}

// ---------- attack 4: picking friends as vouchers ----------

#[test]
fn claimant_is_never_their_own_voucher_and_draw_follows_seed() {
    let mut seen = std::collections::BTreeSet::new();
    for s in 0..20u8 {
        let mut w = world(12, 1);
        w.fund_all(0, 10);
        at_seed(period_start(3), &m(0), [s; 32]);
        let v = w.c.report_sick();
        assert_eq!(v.len(), 2);
        assert!(!v.contains(&m(0)));
        assert_ne!(v[0], v[1]);
        seen.insert(v);
    }
    // The claimant can't steer the pick, and different seeds give different pairs.
    assert!(seen.len() > 5, "draw didn't vary with the seed: {:?}", seen.len());
}

#[test]
#[should_panic(expected = "Not enough eligible members to draw vouchers")]
fn same_voucher_cannot_vouch_for_same_claimant_within_cooldown() {
    // With 3 members the only possible vouchers are the other two. After they vouch once,
    // the cooldown means a fresh report can't be confirmed by the same pair.
    let mut w = world(3, 1);
    w.fund_all(0, 20);
    w.sick_and_confirmed(period_start(3), &m(0));
    at(period_start(3) + 10, &m(0));
    w.c.report_recovered();
    at(period_start(4), &m(0));
    w.c.report_sick();
}

#[test]
fn cooldown_ends_after_twelve_periods() {
    let mut w = world(3, 1);
    w.fund_all(0, 40);
    w.sick_and_confirmed(period_start(3), &m(0));
    at(period_start(3) + 10, &m(0));
    w.c.report_recovered();
    at(period_start(15), &m(0));
    assert_eq!(w.c.report_sick().len(), 2);
}

#[test]
#[should_panic(expected = "The vouch window has passed")]
fn vouch_after_deadline_is_rejected() {
    let mut w = world(5, 1);
    w.fund_all(0, 10);
    at(period_start(3), &m(0));
    let v = w.c.report_sick();
    at(period_start(3) + 61, &v[0]);
    w.c.vouch(m(0));
}

#[test]
fn redraw_replaces_only_missing_vouchers_and_never_repeats() {
    let mut w = world(8, 1);
    w.fund_all(0, 10);
    at(period_start(3), &m(0));
    let first = w.c.report_sick();
    at(period_start(3) + 5, &first[0]);
    w.c.vouch(m(0));
    at(period_start(3) + 61, &acc("anyone.test.near"));
    let now = w.c.redraw(m(0));
    assert_eq!(now.len(), 2);
    assert_eq!(now[0], first[0]);
    assert_ne!(now[1], first[1]);
    assert_ne!(now[1], m(0));
}

#[test]
#[should_panic(expected = "The vouch window is still open")]
fn redraw_waits_for_the_window() {
    let mut w = world(5, 1);
    w.fund_all(0, 10);
    at(period_start(3), &m(0));
    w.c.report_sick();
    at(period_start(3) + 30, &m(0));
    w.c.redraw(m(0));
}

// ---------- attack 5: leaving when a friend gets sick ----------

#[test]
fn leaver_still_pays_the_current_period() {
    let mut w = world(5, 1);
    w.fund_all(0, 20);
    w.sick_and_confirmed(period_start(3), &m(0));
    // m(4) tries to leave in period 4, the first gift period.
    at(period_start(4) + 5, &m(4));
    w.c.request_exit();
    let pot_before = w.pot(&m(4));
    w.settle_through(period_start(5), &m(1));
    // Period 4 gift is split over the 4 healthy members, including the leaver.
    assert_eq!(pot_before - w.pot(&m(4)), UNIT / 4);
    w.check_books();
}

#[test]
#[should_panic(expected = "Nothing to withdraw yet")]
fn pot_is_locked_until_the_exit_period_settles() {
    let mut w = world(5, 1);
    w.fund_all(0, 20);
    at(period_start(4), &m(4));
    w.c.request_exit();
    w.settle_through(period_start(4) + 10, &m(1)); // settles up to period 3 only
    at(period_start(4) + 11, &m(4));
    let _ = w.c.withdraw();
}

#[test]
fn exit_returns_the_pot_after_settlement() {
    let mut w = world(5, 1);
    w.fund_all(0, 20);
    at(period_start(4), &m(4));
    w.c.request_exit();
    w.settle_through(period_start(5), &m(1));
    let pot = w.pot(&m(4));
    assert!(pot > 0);
    at(period_start(5) + 1, &m(4));
    let _ = w.c.withdraw();
    assert_eq!(w.pot(&m(4)), 0);
    assert_eq!(w.c.get_member(m(4)).unwrap().status, Status::Left);
    w.check_books();
    // A member who has left no longer donates.
    w.sick_and_confirmed(period_start(5) + 2, &m(0));
    let s = w.settle_through(period_start(8), &m(1));
    assert!(s.iter().all(|x| x.donors == 3 || x.gifts.is_empty()));
}

// ---------- attack 6: underfunding ----------

#[test]
#[should_panic(expected = "Your deposits aren't up to date")]
fn member_behind_on_deposits_cannot_report() {
    let mut w = world(5, 1);
    w.fund_all(0, 1); // only period 0 paid
    at(period_start(3), &m(0));
    w.c.report_sick();
}

#[test]
fn gifts_scale_down_evenly_when_pots_run_low() {
    // 5 members at level 1, each pot holds only 2 periods of deposits (25 tUSD).
    // Two members sick: demand 500 tUSD, the 3 healthy pots can give at most 75.
    let mut w = world(5, 1);
    w.fund_all(0, 2);
    // Bring deposits up to date for the claimants without growing healthy pots.
    w.deposit(0, &m(0), UNIT / 20 * 3);
    w.deposit(0, &m(1), UNIT / 20 * 3);
    w.sick_and_confirmed(period_start(2), &m(0));
    w.sick_and_confirmed(period_start(2) + 5, &m(1));
    let s = w.settle_through(period_start(4), &m(4));
    let p3 = s.iter().find(|x| x.period == 3).unwrap();
    assert_eq!(p3.demand.0, 2 * UNIT);
    let pots_left: u128 = (2..5).map(|i| w.pot(&m(i))).sum();
    assert_eq!(pots_left, 0, "healthy pots should be emptied");
    assert_eq!(p3.collected.0, 3 * (UNIT / 20 * 2));
    assert!(p3.coverage_bps < 10_000);
    // Both claimants are at the same level, so they get the same scaled gift.
    assert_eq!(p3.gifts[0].paid, p3.gifts[1].paid);
    assert_eq!(p3.gifts.iter().map(|g| g.paid.0).sum::<u128>(), p3.collected.0);
    w.check_books();
}

#[test]
fn full_coverage_pays_exactly_the_gift_and_splits_by_level() {
    let mut w = world(3, 1);
    // Make levels 1, 2, 4 by admitting nobody: rewrite founders directly for the test.
    w.c.members.get_mut(&m(1)).unwrap().level = 2;
    w.c.members.get_mut(&m(2)).unwrap().level = 4;
    // Add more donors so there are two eligible vouchers besides the claimant.
    w.fund_all(0, 40);
    w.sick_and_confirmed(period_start(3), &m(0));
    let p1 = w.pot(&m(1));
    let p2 = w.pot(&m(2));
    let s = w.settle_through(period_start(5), &m(2));
    let p4 = s.iter().find(|x| x.period == 4).unwrap();
    assert_eq!(p4.coverage_bps, 10_000);
    assert_eq!(p4.gifts[0].paid.0, UNIT);
    // Donors at level 2 and 4 pay 1/3 and 2/3 of the gift.
    // 250 tUSD split 2:4 is 83.333333 and 166.666667; the spare unit goes to the larger remainder.
    assert_eq!(p1 - w.pot(&m(1)), 83_333_333);
    assert_eq!(p2 - w.pot(&m(2)), 166_666_667);
    w.check_books();
}

// ---------- waiting period and the 24-period cap ----------

#[test]
fn no_gift_in_the_waiting_period() {
    let mut w = world(5, 1);
    w.fund_all(0, 20);
    w.sick_and_confirmed(period_start(3), &m(0));
    let s = w.settle_through(period_start(4), &m(1));
    let p3 = s.iter().find(|x| x.period == 3).unwrap();
    assert!(p3.gifts.is_empty(), "period of the report is own risk");
    let s = w.settle_through(period_start(5), &m(1));
    assert_eq!(s[0].period, 4);
    assert_eq!(s[0].gifts[0].paid.0, UNIT);
}

#[test]
fn gifts_stop_after_24_periods() {
    let mut w = world(4, 1);
    w.fund_all(0, 400);
    w.sick_and_confirmed(period_start(3), &m(0));
    w.settle_through(period_start(40), &m(1));
    assert_eq!(w.gifts(&m(0)), 24 * UNIT);
    assert!(w.c.get_report(m(0)).is_none(), "report should be closed after 24 gifts");
    w.check_books();
}

#[test]
fn recovery_stops_gifts_from_that_period() {
    let mut w = world(5, 1);
    w.fund_all(0, 20);
    w.sick_and_confirmed(period_start(3), &m(0));
    w.settle_through(period_start(6), &m(1)); // gifts for 4 and 5
    at(period_start(6) + 3, &m(0));
    w.c.report_recovered();
    w.settle_through(period_start(9), &m(1));
    assert_eq!(w.gifts(&m(0)), 2 * UNIT);
    assert!(w.c.get_report(m(0)).is_none());
}

#[test]
fn sick_member_does_not_donate_to_others() {
    let mut w = world(6, 1);
    w.fund_all(0, 20);
    w.sick_and_confirmed(period_start(3), &m(0));
    w.sick_and_confirmed(period_start(3) + 10, &m(1));
    let pot0 = w.pot(&m(0));
    let s = w.settle_through(period_start(5), &m(2));
    assert_eq!(w.pot(&m(0)), pot0);
    assert_eq!(s.last().unwrap().donors, 4);
}

// ---------- attack 7: settling twice ----------

#[test]
#[should_panic(expected = "That period hasn't finished yet")]
fn settle_twice_in_one_period_fails() {
    let mut w = world(3, 1);
    at(period_start(1) + 1, &m(0));
    w.c.settle();
    w.c.settle();
}

#[test]
#[should_panic(expected = "That period hasn't finished yet")]
fn current_period_cannot_be_settled() {
    let mut w = world(3, 1);
    at(10, &m(0));
    w.c.settle();
}

#[test]
fn settle_is_permissionless_and_sequential() {
    let mut w = world(3, 1);
    let s = w.settle_through(period_start(4) + 1, &acc("random-keeper.test.near"));
    assert_eq!(s.iter().map(|x| x.period).collect::<Vec<_>>(), vec![0, 1, 2, 3]);
    assert_eq!(s[0].settled_by, acc("random-keeper.test.near"));
}

// ---------- attack 8: spam ----------

#[test]
#[should_panic(expected = "Not a member")]
fn outsider_cannot_report() {
    let mut w = world(3, 1);
    at(period_start(3), &acc("stranger.test.near"));
    w.c.report_sick();
}

#[test]
#[should_panic(expected = "You already have an open report")]
fn one_open_report_per_member() {
    let mut w = world(5, 1);
    w.fund_all(0, 10);
    at(period_start(3), &m(0));
    w.c.report_sick();
    w.c.report_sick();
}

// ---------- deposits ----------

#[test]
#[should_panic(expected = "Only the circle's token is accepted")]
fn fake_token_deposit_is_rejected() {
    let mut w = world(3, 1);
    at(10, &acc("fake-token.test.near"));
    let _ = w.c.ft_on_transfer(m(0), U128(1_000_000_000), String::new());
}

#[test]
fn outsider_deposit_is_refunded() {
    let mut w = world(3, 1);
    assert_eq!(w.deposit(10, &acc("stranger.test.near"), 5_000_000), 5_000_000);
    assert_eq!(w.c.get_held().0, 0);
}

#[test]
fn exiting_member_deposit_is_refunded() {
    let mut w = world(3, 1);
    at(10, &m(2));
    w.c.request_exit();
    assert_eq!(w.deposit(11, &m(2), 5_000_000), 5_000_000);
}

// ---------- admission with veto ----------

#[test]
#[should_panic(expected = "Proposal isn't open")]
fn one_veto_blocks_admission() {
    let mut w = world(5, 1);
    at(10, &m(0));
    let id = w.c.propose(ProposalKind::Admit { account_id: acc("x.test.near"), level: 1 });
    for i in 1..5 {
        at(11, &m(i));
        if i == 4 {
            w.c.veto(id);
        } else {
            w.c.approve(id);
        }
    }
    at(400, &m(0));
    w.c.execute(id);
}

#[test]
#[should_panic(expected = "Veto window still open")]
fn admission_waits_for_the_veto_window() {
    let mut w = world(3, 1);
    at(10, &m(0));
    let id = w.c.propose(ProposalKind::Admit { account_id: acc("x.test.near"), level: 1 });
    at(11, &m(1));
    w.c.approve(id);
    at(12, &m(2));
    w.c.execute(id);
}

#[test]
#[should_panic(expected = "Proposal expired")]
fn proposals_expire() {
    let mut w = world(3, 1);
    at(10, &m(0));
    let id = w.c.propose(ProposalKind::Admit { account_id: acc("x.test.near"), level: 1 });
    at(11, &m(1));
    w.c.approve(id);
    at(10 + 600, &m(2));
    w.c.execute(id);
}

#[test]
fn admission_by_majority_after_window() {
    let mut w = world(5, 1);
    let x = acc("x.test.near");
    at(10, &m(0));
    let id = w.c.propose(ProposalKind::Admit { account_id: x.clone(), level: 3 });
    for i in 1..3 {
        at(11, &m(i));
        w.c.approve(id);
    }
    at(320, &acc("anyone.test.near"));
    w.c.execute(id);
    let v = w.c.get_member(x).unwrap();
    assert_eq!(v.level, 3);
    assert_eq!(v.status, Status::Active);
    assert_eq!(v.joined_period.0, 2);
}

#[test]
#[should_panic(expected = "Only admissions can be vetoed")]
fn veto_only_applies_to_admissions() {
    let mut w = world(5, 1);
    at(10, &m(1));
    let id = w.c.propose(ProposalKind::Expel { account_id: m(0) });
    at(11, &m(2));
    w.c.veto(id);
}

// ---------- withdrawals ----------

#[test]
fn failed_withdrawal_puts_everything_back() {
    let mut w = world(5, 1);
    w.fund_all(0, 20);
    w.sick_and_confirmed(period_start(3), &m(0));
    w.settle_through(period_start(5), &m(1));
    let g = w.gifts(&m(0));
    assert_eq!(g, UNIT);
    at(period_start(5) + 1, &m(0));
    let _ = w.c.withdraw();
    assert_eq!(w.gifts(&m(0)), 0);
    testing_env!(
        VMContextBuilder::new()
            .current_account_id(circle_id())
            .predecessor_account_id(circle_id())
            .block_timestamp(T0 + (period_start(5) + 2) * SEC)
            .build(),
        near_sdk::test_vm_config(),
        RuntimeFeesConfig::test(),
        Default::default(),
        vec![PromiseResult::Failed],
    );
    let ok = w.c.on_withdraw(m(0), U128(g), U128(0), Status::Active, Err(PromiseError::Failed));
    assert!(!ok);
    assert_eq!(w.gifts(&m(0)), g);
    w.check_books();
}

#[test]
fn failed_exit_withdrawal_restores_status_and_pot() {
    let mut w = world(5, 1);
    w.fund_all(0, 20);
    at(period_start(1), &m(4));
    w.c.request_exit();
    w.settle_through(period_start(2), &m(1));
    let pot = w.pot(&m(4));
    at(period_start(2) + 1, &m(4));
    let _ = w.c.withdraw();
    assert_eq!(w.c.get_member(m(4)).unwrap().status, Status::Left);
    at(period_start(2) + 2, &circle_id());
    w.c.on_withdraw(m(4), U128(0), U128(pot), Status::Exiting { after_period: 1 }, Err(PromiseError::Failed));
    assert_eq!(w.pot(&m(4)), pot);
    assert_eq!(w.c.get_member(m(4)).unwrap().status, Status::Exiting { after_period: 1 });
    w.check_books();
}

#[test]
#[should_panic(expected = "Only the circle can call this")]
fn withdraw_callback_is_private() {
    let mut w = world(3, 1);
    at(10, &m(0));
    w.c.on_withdraw(m(0), U128(1), U128(0), Status::Active, Ok(()));
}

// ---------- money is conserved through a long busy run ----------

#[test]
fn books_balance_through_a_busy_year() {
    let mut w = world(20, 1);
    for i in 0..20 {
        w.c.members.get_mut(&m(i)).unwrap().level = (i % 8 + 1) as u8;
    }
    w.fund_all(0, 3);
    let mut t = period_start(3);
    for p in 3..15u64 {
        t = period_start(p) + 1;
        w.fund_all(t, 1);
        if p % 3 == 0 {
            let who = m((p as usize * 7) % 20);
            if w.c.get_report(who.clone()).is_none() && w.c.get_member(who.clone()).unwrap().good_standing {
                w.sick_and_confirmed(t + 1, &who);
            }
        }
        if p % 4 == 0 {
            if let Some(r) = w.c.get_reports().first() {
                let a = r.account_id.clone();
                at(t + 3, &a);
                w.c.report_recovered();
            }
        }
        w.settle_through(t + 5, &m(19));
        w.check_books();
    }
    let _ = t;
}

// ---------- added in v2 ----------

#[test]
#[should_panic(expected = "Attach 0.01 NEAR")]
fn proposal_without_bond_is_rejected() {
    let mut w = world(5, 1);
    testing_env!(VMContextBuilder::new()
        .current_account_id(circle_id())
        .predecessor_account_id(m(0))
        .block_timestamp(T0 + 10 * SEC)
        .attached_deposit(NearToken::from_millinear(9))
        .build());
    w.c.propose(ProposalKind::Admit { account_id: acc("friend.test.near"), level: 1 });
}

#[test]
fn leaver_with_an_empty_pot_can_still_leave() {
    let mut w = world(5, 1);
    // m4 never deposits, asks to leave, and has nothing to take back.
    at(10, &m(4));
    w.c.request_exit();
    at(11, &m(4));
    let early = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = w.c.withdraw();
    }));
    assert!(early.is_err(), "can't finish leaving before the exit period settles");
    w.settle_through(period_start(1) + 1, &m(1));
    at(period_start(1) + 2, &m(4));
    match w.c.withdraw() {
        PromiseOrValue::Value(true) => {}
        _ => panic!("expected an immediate exit with nothing to send"),
    }
    assert_eq!(w.c.get_member(m(4)).unwrap().status, Status::Left);
    // Having left, they can be admitted again later.
    at(period_start(1) + 3, &m(0));
    w.c.propose(ProposalKind::Admit { account_id: m(4), level: 1 });
    w.check_books();
}
