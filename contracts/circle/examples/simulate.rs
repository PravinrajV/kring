//! Kring simulation: 20 members, 24 monthly periods, three labeled sickness scenarios.
//!
//! This runs the real circle contract code (the same crate that compiles to wasm) inside
//! near-sdk's mocked blockchain. Only the sickness pattern is invented, and it's stated
//! up front as an assumption. Same seed, same output, every time.
//!
//!   cargo run --example simulate -- [seed]
//!
//! Writes sim/<scenario>.csv and sim/SIMULATION.md at the repo root.

use kring_circle::{Circle, Config, FounderInput};
use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_sdk::json_types::U128;
use near_sdk::test_utils::VMContextBuilder;
use near_sdk::{testing_env, AccountId};
use std::fmt::Write as _;
use std::path::PathBuf;

const MEMBERS: usize = 20;
const PERIODS: u64 = 24;
const MONTH: u64 = 30 * 24 * 3600; // one period = 30 days
const SEC: u64 = 1_000_000_000;
const T0: u64 = 1_767_225_600 * SEC; // 2026-01-01 00:00 UTC, only a label for the clock
const UNIT: u128 = 500_000_000; // 500 tUSD a month per level (6 decimals)

struct Scenario {
    key: &'static str,
    title: &'static str,
    onset: f64,        // chance a healthy member falls long-term sick in a given month
    mean_months: f64,  // average spell length; each month a sick member recovers with 1/mean
    forced: &'static [(usize, u64)], // (member index, month) who fall sick and stay sick
    note: &'static str,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        key: "baseline",
        title: "A. Baseline",
        onset: 0.01,
        mean_months: 4.0,
        forced: &[],
        note: "1% of healthy members start a long spell each month; spells average 4 months.",
    },
    Scenario {
        key: "bad-years",
        title: "B. Two bad years",
        onset: 0.03,
        mean_months: 6.0,
        forced: &[],
        note: "3% a month, spells average 6 months. Roughly three times the baseline load.",
    },
    Scenario {
        key: "shock",
        title: "C. Shock",
        onset: 0.01,
        mean_months: 4.0,
        forced: &[(0, 3), (5, 3), (10, 3), (15, 3)],
        note: "Baseline, plus four members (one per level) fall sick in month 3 and stay sick.",
    },
];

/// splitmix64: tiny, deterministic, good enough for a reproducible sickness pattern.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p && p > 0.0
    }
    fn seed32(&mut self) -> [u8; 32] {
        let mut s = [0u8; 32];
        for c in s.chunks_mut(8) {
            c.copy_from_slice(&self.next().to_le_bytes());
        }
        s
    }
}

fn acc(s: &str) -> AccountId {
    s.parse().unwrap()
}
fn member(i: usize) -> AccountId {
    acc(&format!("m{:02}.sim", i + 1))
}
fn level(i: usize) -> u8 {
    (i / 5) as u8 + 1 // five members on each of levels 1 to 4
}
fn tusd(u: u128) -> f64 {
    u as f64 / 1e6
}

fn at(secs: u64, who: &AccountId, seed: [u8; 32]) {
    testing_env!(VMContextBuilder::new()
        .current_account_id(acc("circle.sim"))
        .predecessor_account_id(who.clone())
        .block_timestamp(T0 + secs * SEC)
        .random_seed(seed)
        .build());
}

struct Row {
    period: u64,
    sick: usize,
    paid_recipients: usize,
    demand: u128,
    collected: u128,
    coverage_bps: u32,
    donors: u32,
    empty_pots: usize,
    pots_after: u128,
    min_pot: u128,
}

fn run(s: &Scenario, seed: u64) -> Vec<Row> {
    near_sdk::mock::with_mocked_blockchain(|b| {
        b.take_storage();
    });
    let mut rng = Rng(seed ^ s.key.bytes().fold(0u64, |a, b| a.wrapping_mul(31).wrapping_add(b as u64)));
    let token = acc("tusd.sim");
    let keeper = acc("keeper.sim");
    let config = Config {
        period_secs: MONTH as u32,
        waiting_periods: 1,
        probation_periods: 1,
        max_gift_periods: 24,
        vouch_window_secs: 7 * 24 * 3600,
        vouch_cooldown_periods: 12,
        proposal_secs: 7 * 24 * 3600,
        gift_unit: U128(UNIT),
    };
    at(0, &acc("circle.sim"), rng.seed32());
    let founders = (0..MEMBERS).map(|i| FounderInput { account_id: member(i), level: level(i) }).collect();
    let mut c = Circle::new(token.clone(), founders, config);

    let mut sick = vec![false; MEMBERS];
    let mut stays_sick = vec![false; MEMBERS];
    let mut rows = vec![];
    let settle_upto = |c: &mut Circle, secs: u64, seed: [u8; 32], sick_now: usize, rows: &mut Vec<Row>| loop {
        at(secs, &keeper, seed);
        if !c.get_circle().settle_ready {
            break;
        }
        let st = c.settle();
        let pots: Vec<u128> = c.get_members().iter().filter(|m| !m.expelled).map(|m| m.pot.0).collect();
        rows.push(Row {
            period: st.period,
            sick: sick_now,
            paid_recipients: st.gifts.iter().filter(|g| g.paid.0 > 0).count(),
            demand: st.demand.0,
            collected: st.collected.0,
            coverage_bps: st.coverage_bps,
            donors: st.donors,
            empty_pots: pots.iter().filter(|p| **p == 0).count(),
            pots_after: st.pots_after.0,
            min_pot: pots.iter().copied().min().unwrap_or(0),
        });
    };

    for month in 0..PERIODS {
        let t = month * MONTH;
        let sick_last_month = sick.iter().filter(|x| **x).count();
        settle_upto(&mut c, t + 5, rng.seed32(), sick_last_month, &mut rows);

        // Everyone pays this month's deposit, sick or not (assumption: sick members keep paying).
        for i in 0..MEMBERS {
            let due = UNIT * level(i) as u128 / 20;
            at(t + 10, &token, rng.seed32());
            let _ = c.ft_on_transfer(member(i), U128(due), String::new());
        }
        // Recoveries first, then new spells.
        for i in 0..MEMBERS {
            if sick[i] && !stays_sick[i] && rng.chance(1.0 / s.mean_months) {
                at(t + 20, &member(i), rng.seed32());
                c.report_recovered();
                sick[i] = false;
            }
        }
        for i in 0..MEMBERS {
            let forced = s.forced.iter().any(|(m, when)| *m == i && *when == month);
            let random = month >= 1 && rng.chance(s.onset);
            if !sick[i] && (forced || random) {
                at(t + 30, &member(i), rng.seed32());
                let vouchers = c.report_sick();
                for v in vouchers {
                    at(t + 60, &v, rng.seed32());
                    c.vouch(member(i));
                }
                sick[i] = true;
                stays_sick[i] = forced;
            }
        }
    }
    let sick_end = sick.iter().filter(|x| **x).count();
    settle_upto(&mut c, PERIODS * MONTH + 5, rng.seed32(), sick_end, &mut rows);

    // The books must balance exactly after every scenario.
    let view = c.get_circle();
    let pots: u128 = c.get_members().iter().map(|m| m.pot.0).sum();
    let gifts: u128 = c.get_members().iter().map(|m| m.gift_balance.0).sum();
    let deposited: u128 = c.get_members().iter().map(|m| m.deposited.0).sum();
    assert_eq!(pots, view.total_pots.0);
    assert_eq!(gifts, view.total_gift_balances.0);
    assert_eq!(c.get_held().0, pots + gifts);
    assert_eq!(deposited, pots + gifts, "every deposited unit is either in a pot or owed as a gift");
    rows
}

fn main() {
    let seed: u64 = std::env::args().nth(1).map(|s| s.parse().expect("seed must be a number")).unwrap_or(2026);
    let out = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sim");
    std::fs::create_dir_all(&out).unwrap();

    let mut md = String::new();
    writeln!(md, "# Kring simulation (seed {seed})\n").unwrap();
    writeln!(md, "**This is a simulation, not data.** It runs the real circle contract code in near-sdk's mocked blockchain. The sickness patterns below are assumptions picked to stress the circle, not statistics.\n").unwrap();
    writeln!(md, "Setup: {MEMBERS} members, five on each level 1 to 4, gift {} to {} tUSD a month, deposit 1/20 of the gift every month (the ratio Broodfonds uses), one-month waiting period, gifts for at most 24 months, monthly settlement, pots start empty. Sick members keep paying their deposit. Vouchers always vouch.\n", tusd(UNIT), tusd(UNIT * 4)).unwrap();
    writeln!(md, "Coverage is the share of the month's gifts the healthy members' pots could pay. Below 100%, every gift is scaled down by the same factor rather than anyone's pot going negative.\n").unwrap();
    writeln!(md, "Reproduce: `cargo run --example simulate -- {seed}` from `contracts/circle`.\n").unwrap();
    writeln!(md, "| Scenario | Assumption | Spells | Gifts due (tUSD) | Gifts paid (tUSD) | Months below 100% | Lowest coverage | Empty pots at worst | Pots left (tUSD) |").unwrap();
    writeln!(md, "|---|---|---|---|---|---|---|---|---|").unwrap();
    let mut detail = String::new();

    for s in SCENARIOS {
        let rows = run(s, seed);
        let mut csv = String::from("period,sick_members,gift_recipients,demand_tusd,collected_tusd,coverage_pct,donors,empty_pots,pots_after_tusd,min_pot_tusd\n");
        for r in &rows {
            writeln!(csv, "{},{},{},{:.2},{:.2},{:.2},{},{},{:.2},{:.2}", r.period, r.sick, r.paid_recipients, tusd(r.demand), tusd(r.collected), r.coverage_bps as f64 / 100.0, r.donors, r.empty_pots, tusd(r.pots_after), tusd(r.min_pot)).unwrap();
        }
        std::fs::write(out.join(format!("{}.csv", s.key)), &csv).unwrap();

        let due: u128 = rows.iter().map(|r| r.demand).sum();
        let paid: u128 = rows.iter().map(|r| r.collected).sum();
        let scaled: Vec<&Row> = rows.iter().filter(|r| r.coverage_bps < 10_000).collect();
        let lowest = rows.iter().map(|r| r.coverage_bps).min().unwrap_or(10_000);
        let worst_empty = rows.iter().map(|r| r.empty_pots).max().unwrap_or(0);
        let spells: usize = {
            let mut n = 0;
            let mut prev = 0;
            for r in &rows {
                if r.sick > prev { n += r.sick - prev; }
                prev = r.sick;
            }
            n
        };
        writeln!(md, "| {} | {} | {} | {:.0} | {:.0} | {} | {:.1}% | {} | {:.0} |", s.title, s.note, spells, tusd(due), tusd(paid), scaled.len(), lowest as f64 / 100.0, worst_empty, tusd(rows.last().unwrap().pots_after)).unwrap();

        writeln!(detail, "\n## {}\n\n{}\n", s.title, s.note).unwrap();
        writeln!(detail, "| Month | Sick | Paid | Due (tUSD) | Paid (tUSD) | Coverage | Empty pots | Pots after (tUSD) |").unwrap();
        writeln!(detail, "|---|---|---|---|---|---|---|---|").unwrap();
        for r in &rows {
            writeln!(detail, "| {} | {} | {} | {:.0} | {:.0} | {:.1}% | {} | {:.0} |", r.period, r.sick, r.paid_recipients, tusd(r.demand), tusd(r.collected), r.coverage_bps as f64 / 100.0, r.empty_pots, tusd(r.pots_after)).unwrap();
        }
        println!("{:<18} spells {:>2}  due {:>9.0}  paid {:>9.0}  months scaled down {:>2}  lowest coverage {:>5.1}%", s.title, spells, tusd(due), tusd(paid), scaled.len(), lowest as f64 / 100.0);
    }
    writeln!(md, "\n\"Spells\" counts new sickness spells in the 24 months. \"Sick\" in the tables below is the count at the end of each month's activity, including members still in their waiting month.").unwrap();
    md.push_str(&detail);
    std::fs::write(out.join("SIMULATION.md"), md).unwrap();
    println!("Wrote sim/SIMULATION.md and sim/*.csv");
}
