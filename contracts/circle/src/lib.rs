//! Kring circle: a keyless sick-pay gift circle for freelancers on NEAR.
//!
//! Members park a fixed amount each period in their own pot. When a member is too sick to
//! work and two randomly drawn peers vouch for them, every healthy pot sends a small gift each
//! period, after a waiting period, for at most `max_gift_periods`. If the pots can't cover
//! everything, every gift scales down by the same ratio. Leaving returns what's left.
//!
//! There's no owner and no admin method. The account's keys are deleted after deploy.
//! Admission, expulsion and code upgrades only happen by member vote.

use near_contract_standards::fungible_token::core::ext_ft_core;
use near_contract_standards::fungible_token::receiver::FungibleTokenReceiver;
use near_sdk::json_types::{U128, U64};
use near_sdk::serde_json::{self, json};
use near_sdk::store::{IterableMap, LookupMap, Vector};
use near_sdk::{
    env, log, near, require, AccountId, BorshStorageKey, Gas, NearToken, PanicOnDefault,
    Promise, PromiseError, PromiseOrValue,
};

pub mod math;
#[cfg(test)]
mod tests;

use math::{apportion, coverage_bps};

pub const MAX_MEMBERS: usize = 50;
pub const MIN_FOUNDERS: usize = 3;
pub const MAX_LEVEL: u8 = 8;
/// Hard cap from the offline circles this copies: gifts run for at most two years.
pub const MAX_GIFT_PERIODS_CAP: u32 = 24;
/// A level's deposit per period is its gift divided by this.
pub const GIFT_TO_DEPOSIT: u128 = 20;
pub const VOUCHERS_PER_REPORT: usize = 2;
/// Attached to every proposal and kept by the circle. It pays for the proposal's storage
/// (about 1 KB at 0.01 NEAR), so nobody can drain the circle's NEAR by spamming proposals.
pub const PROPOSAL_BOND: NearToken = NearToken::from_millinear(10);

const GAS_FT_TRANSFER: Gas = Gas::from_tgas(15);
const GAS_CALLBACK: Gas = Gas::from_tgas(15);

#[derive(BorshStorageKey)]
#[near]
enum Key {
    Members,
    Reports,
    LastVouch,
    Settlements,
    Proposals,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    /// Length of one period. A month in production, a couple of minutes in the demo.
    pub period_secs: u32,
    /// Periods after a report before gifts start. The first month is the member's own risk.
    pub waiting_periods: u32,
    /// Periods a new member must wait before they can report sick.
    pub probation_periods: u32,
    /// Gift periods per report, at most 24.
    pub max_gift_periods: u32,
    /// Time the two drawn vouchers have to vouch before anyone may redraw.
    pub vouch_window_secs: u32,
    /// A member can't vouch for the same claimant again within this many periods.
    pub vouch_cooldown_periods: u32,
    /// Admission proposals wait this long for vetoes; every proposal expires after twice this.
    pub proposal_secs: u32,
    /// Gift per period for level 1, in token units. Level n gets n times this.
    pub gift_unit: U128,
}

impl Config {
    fn validate(&self) {
        require!(self.period_secs >= 60, "period_secs must be at least 60");
        require!(self.waiting_periods >= 1, "waiting_periods must be at least 1");
        require!(
            self.max_gift_periods >= 1 && self.max_gift_periods <= MAX_GIFT_PERIODS_CAP,
            "max_gift_periods must be 1 to 24"
        );
        require!(self.vouch_window_secs >= 60, "vouch_window_secs must be at least 60");
        require!(self.vouch_cooldown_periods >= 1, "vouch_cooldown_periods must be at least 1");
        require!(self.proposal_secs >= 60, "proposal_secs must be at least 60");
        require!(
            self.gift_unit.0 >= GIFT_TO_DEPOSIT && self.gift_unit.0 % GIFT_TO_DEPOSIT == 0,
            "gift_unit must be a positive multiple of 20"
        );
        require!(self.gift_unit.0 < 1u128 << 64, "gift_unit too large");
    }
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Active,
    /// Leaves once `after_period` has been settled.
    Exiting { after_period: u64 },
    Left,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Member {
    pub level: u8,
    /// Own money still in the circle. Only gifts or an exit move it.
    pub pot: u128,
    /// Everything this member ever deposited since they last joined. Used for good standing.
    pub deposited: u128,
    pub joined_period: u64,
    pub status: Status,
    /// Gifts received and not yet withdrawn.
    pub gift_balance: u128,
    pub expelled: bool,
}

#[near(serializers = [borsh])]
#[derive(Clone, Debug)]
pub struct Report {
    pub opened_period: u64,
    pub opened_ns: u64,
    pub vouch_deadline_ns: u64,
    pub vouchers: Vec<AccountId>,
    pub vouched: Vec<AccountId>,
    /// Everyone ever drawn for this report, so a redraw never picks the same person twice.
    pub drawn: Vec<AccountId>,
    pub confirmed_period: Option<u64>,
    pub gifts_paid: u32,
    /// No gifts for this period or later.
    pub closes_from: Option<u64>,
}

impl Report {
    fn sick_in(&self, period: u64) -> bool {
        matches!(self.confirmed_period, Some(c) if c <= period)
            && self.closes_from.map_or(true, |c| period < c)
    }
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct GiftLine {
    pub account_id: AccountId,
    pub due: U128,
    pub paid: U128,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Settlement {
    pub period: u64,
    pub demand: U128,
    pub collected: U128,
    pub coverage_bps: u32,
    pub donors: u32,
    pub gifts: Vec<GiftLine>,
    pub pots_after: U128,
    pub settled_by: AccountId,
    pub settled_at_ns: U64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProposalKind {
    Admit { account_id: AccountId, level: u8 },
    Expel { account_id: AccountId },
    /// Hex sha256 of the new wasm. The code itself is sent to `execute_upgrade`.
    Upgrade { code_hash: String },
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProposalStatus {
    Open,
    Executed,
    Vetoed,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: u64,
    pub kind: ProposalKind,
    pub proposer: AccountId,
    pub created_ns: U64,
    pub approvals: Vec<AccountId>,
    pub vetoed_by: Option<AccountId>,
    pub status: ProposalStatus,
}

#[near(serializers = [json])]
pub struct FounderInput {
    pub account_id: AccountId,
    pub level: u8,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Circle {
    token: AccountId,
    config: Config,
    genesis_ns: u64,
    members: IterableMap<AccountId, Member>,
    reports: IterableMap<AccountId, Report>,
    last_vouch: LookupMap<(AccountId, AccountId), u64>,
    settlements: Vector<Settlement>,
    proposals: Vector<Proposal>,
    next_to_settle: u64,
    total_pots: u128,
    total_gift_balances: u128,
    draw_nonce: u64,
}

// ---------- views (JSON shapes) ----------

#[near(serializers = [json])]
pub struct MemberView {
    pub account_id: AccountId,
    pub level: u8,
    pub pot: U128,
    pub deposited: U128,
    pub required_deposit_to_date: U128,
    pub good_standing: bool,
    pub joined_period: U64,
    pub status: Status,
    pub gift_balance: U128,
    pub gift_per_period: U128,
    pub deposit_per_period: U128,
    pub expelled: bool,
}

#[near(serializers = [json])]
pub struct ReportView {
    pub account_id: AccountId,
    pub opened_period: U64,
    pub opened_ns: U64,
    pub vouch_deadline_ns: U64,
    pub vouchers: Vec<AccountId>,
    pub vouched: Vec<AccountId>,
    pub confirmed_period: Option<U64>,
    pub first_gift_period: U64,
    pub gifts_paid: u32,
    pub closes_from: Option<U64>,
}

#[near(serializers = [json])]
pub struct CircleView {
    pub token: AccountId,
    pub config: Config,
    pub genesis_ns: U64,
    pub now_ns: U64,
    pub current_period: U64,
    pub next_to_settle: U64,
    pub settle_ready: bool,
    pub active_members: u32,
    pub exiting_members: u32,
    pub open_reports: u32,
    pub total_pots: U128,
    pub total_gift_balances: U128,
    pub last_coverage_bps: Option<u32>,
    pub proposals: u64,
}

fn emit(event: &str, data: serde_json::Value) {
    log!(
        "EVENT_JSON:{}",
        json!({"standard": "kring", "version": "1.0.0", "event": event, "data": [data]})
    );
}

#[near]
impl Circle {
    /// Called once, by the circle account itself, in the same transaction as the deploy.
    /// After this the deployer deletes every access key, so nobody holds the circle.
    #[init]
    #[private]
    pub fn new(token: AccountId, founders: Vec<FounderInput>, config: Config) -> Self {
        // #[private] already enforces this on chain; checked here too so it's explicit and tested.
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "Only the circle account can initialize itself"
        );
        config.validate();
        require!(
            founders.len() >= MIN_FOUNDERS && founders.len() <= MAX_MEMBERS,
            "A circle needs 3 to 50 founders"
        );
        let mut members = IterableMap::new(Key::Members);
        for f in founders {
            require!(f.level >= 1 && f.level <= MAX_LEVEL, "level must be 1 to 8");
            require!(f.account_id != env::current_account_id(), "The circle can't be a member");
            require!(!members.contains_key(&f.account_id), "Duplicate founder");
            members.insert(
                f.account_id,
                Member {
                    level: f.level,
                    pot: 0,
                    deposited: 0,
                    joined_period: 0,
                    status: Status::Active,
                    gift_balance: 0,
                    expelled: false,
                },
            );
        }
        let me = Self {
            token,
            config,
            genesis_ns: env::block_timestamp(),
            members,
            reports: IterableMap::new(Key::Reports),
            last_vouch: LookupMap::new(Key::LastVouch),
            settlements: Vector::new(Key::Settlements),
            proposals: Vector::new(Key::Proposals),
            next_to_settle: 0,
            total_pots: 0,
            total_gift_balances: 0,
            draw_nonce: 0,
        };
        emit("circle_created", json!({"founders": me.members.len(), "token": me.token}));
        me
    }

    // ---------- time ----------

    pub fn current_period(&self) -> U64 {
        U64(self.period_at(env::block_timestamp()))
    }

    fn period_at(&self, ns: u64) -> u64 {
        (ns.saturating_sub(self.genesis_ns)) / (self.config.period_secs as u64 * 1_000_000_000)
    }

    fn now_period(&self) -> u64 {
        self.period_at(env::block_timestamp())
    }

    // ---------- helpers ----------

    fn gift_of(&self, level: u8) -> u128 {
        self.config.gift_unit.0 * level as u128
    }

    fn deposit_of(&self, level: u8) -> u128 {
        self.gift_of(level) / GIFT_TO_DEPOSIT
    }

    fn required_to_date(&self, m: &Member) -> u128 {
        let periods = self.now_period().saturating_sub(m.joined_period) + 1;
        self.deposit_of(m.level) * periods as u128
    }

    fn active_member(&self, who: &AccountId) -> &Member {
        let m = self.members.get(who).unwrap_or_else(|| env::panic_str("Not a member"));
        require!(m.status == Status::Active, "Only active members can do this");
        m
    }

    fn is_active(&self, who: &AccountId) -> bool {
        matches!(self.members.get(who), Some(m) if m.status == Status::Active)
    }

    fn draw(&mut self, claimant: &AccountId, exclude: &[AccountId], n: usize) -> Vec<AccountId> {
        let period = self.now_period();
        let cooldown = self.config.vouch_cooldown_periods as u64;
        let mut pool: Vec<AccountId> = self
            .members
            .iter()
            .filter(|(a, m)| {
                m.status == Status::Active && *a != claimant && !exclude.contains(a)
            })
            .map(|(a, _)| a.clone())
            .filter(|a| !self.reports.contains_key(a))
            .filter(|a| match self.last_vouch.get(&(a.clone(), claimant.clone())) {
                Some(p) => period.saturating_sub(*p) >= cooldown,
                None => true,
            })
            .collect();
        pool.sort();
        require!(pool.len() >= n, "Not enough eligible members to draw vouchers");
        let seed = env::random_seed();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let mut buf = seed.clone();
            buf.extend_from_slice(claimant.as_bytes());
            buf.extend_from_slice(&self.draw_nonce.to_le_bytes());
            buf.push(i as u8);
            let h = env::sha256(&buf);
            let mut b = [0u8; 8];
            b.copy_from_slice(&h[0..8]);
            let idx = (u64::from_le_bytes(b) % pool.len() as u64) as usize;
            out.push(pool.swap_remove(idx));
        }
        self.draw_nonce += 1;
        out
    }

    // ---------- deposits ----------

    // ft_on_transfer is in the FungibleTokenReceiver impl below.

    // ---------- sickness ----------

    /// Report that you're too sick to work. Draws two vouchers you don't choose.
    pub fn report_sick(&mut self) -> Vec<AccountId> {
        let me = env::predecessor_account_id();
        let m = self.active_member(&me).clone();
        let period = self.now_period();
        if let Some(r) = self.reports.get(&me) {
            require!(r.closes_from.is_some(), "You already have an open report");
            env::panic_str("Your last report is still being settled; try again after the next settle");
        }
        require!(
            period >= m.joined_period + self.config.probation_periods as u64,
            "Still in probation: new members can't report sick yet"
        );
        require!(
            m.deposited >= self.required_to_date(&m),
            "Your deposits aren't up to date"
        );
        let vouchers = self.draw(&me, &[], VOUCHERS_PER_REPORT);
        let now = env::block_timestamp();
        let report = Report {
            opened_period: period,
            opened_ns: now,
            vouch_deadline_ns: now + self.config.vouch_window_secs as u64 * 1_000_000_000,
            vouchers: vouchers.clone(),
            vouched: vec![],
            drawn: vouchers.clone(),
            confirmed_period: None,
            gifts_paid: 0,
            closes_from: None,
        };
        self.reports.insert(me.clone(), report);
        emit("report_opened", json!({"account_id": me, "period": period, "vouchers": vouchers}));
        vouchers
    }

    /// A drawn voucher confirms they've seen the claimant and believe the report.
    pub fn vouch(&mut self, claimant: AccountId) {
        let me = env::predecessor_account_id();
        self.active_member(&me);
        let period = self.now_period();
        let now = env::block_timestamp();
        let r = self
            .reports
            .get_mut(&claimant)
            .unwrap_or_else(|| env::panic_str("No open report for that member"));
        require!(r.closes_from.is_none(), "That report is closed");
        require!(r.vouchers.contains(&me), "You weren't drawn to vouch for this report");
        require!(!r.vouched.contains(&me), "You already vouched");
        require!(now <= r.vouch_deadline_ns, "The vouch window has passed; ask for a redraw");
        r.vouched.push(me.clone());
        let confirmed = r.vouched.len() >= VOUCHERS_PER_REPORT && r.confirmed_period.is_none();
        if confirmed {
            r.confirmed_period = Some(period);
        }
        self.last_vouch.insert((me.clone(), claimant.clone()), period);
        emit("vouched", json!({"account_id": me, "claimant": claimant, "confirmed": confirmed}));
    }

    /// After the vouch window, anyone can replace drawn vouchers who didn't vouch.
    pub fn redraw(&mut self, claimant: AccountId) -> Vec<AccountId> {
        let now = env::block_timestamp();
        let r = self
            .reports
            .get(&claimant)
            .unwrap_or_else(|| env::panic_str("No open report for that member"))
            .clone();
        require!(r.confirmed_period.is_none(), "Already confirmed");
        require!(r.closes_from.is_none(), "That report is closed");
        require!(now > r.vouch_deadline_ns, "The vouch window is still open");
        let missing = VOUCHERS_PER_REPORT - r.vouched.len();
        let fresh = self.draw(&claimant, &r.drawn, missing);
        let window = self.config.vouch_window_secs as u64 * 1_000_000_000;
        let r = self.reports.get_mut(&claimant).unwrap();
        r.vouchers = r.vouched.iter().cloned().chain(fresh.iter().cloned()).collect();
        r.drawn.extend(fresh.iter().cloned());
        r.vouch_deadline_ns = now + window;
        emit("vouchers_redrawn", json!({"claimant": claimant, "new_vouchers": fresh}));
        r.vouchers.clone()
    }

    /// Back at work (or withdrawing an unconfirmed report). Gifts stop from this period.
    pub fn report_recovered(&mut self) {
        let me = env::predecessor_account_id();
        let period = self.now_period();
        let next = self.next_to_settle;
        let waiting = self.config.waiting_periods as u64;
        let r = self
            .reports
            .get_mut(&me)
            .unwrap_or_else(|| env::panic_str("You have no open report"));
        require!(r.closes_from.is_none(), "Already closed");
        r.closes_from = Some(period);
        // Keep the report only while some unsettled period before `period` can still pay a gift.
        let remove_now = match r.confirmed_period {
            None => true,
            Some(c) => {
                let first_payable = next.max(c).max(r.opened_period + waiting);
                first_payable >= period
            }
        };
        if remove_now {
            self.reports.remove(&me);
        }
        emit("report_closed", json!({"account_id": me, "closes_from": period}));
    }

    // ---------- settlement ----------

    /// Settles the next finished period. Anyone can call it, and each period settles once.
    pub fn settle(&mut self) -> Settlement {
        let p = self.next_to_settle;
        require!(self.now_period() > p, "That period hasn't finished yet");

        // Recipients: confirmed, past the waiting period, under the cap, not closed.
        let waiting = self.config.waiting_periods as u64;
        let max_gifts = self.config.max_gift_periods;
        let mut recipients: Vec<(AccountId, u128)> = Vec::new();
        let mut sick: Vec<AccountId> = Vec::new();
        for (a, r) in self.reports.iter() {
            let m = match self.members.get(a) {
                Some(m) if m.status != Status::Left => m,
                _ => continue,
            };
            if !r.sick_in(p) {
                continue;
            }
            sick.push(a.clone());
            if p >= r.opened_period + waiting && r.gifts_paid < max_gifts {
                recipients.push((a.clone(), self.gift_of(m.level)));
            }
        }
        recipients.sort_by(|x, y| x.0.cmp(&y.0));

        // Donors: everyone in the circle during p who isn't sick in p.
        let mut donors: Vec<(AccountId, u128, u128)> = self
            .members
            .iter()
            .filter(|(a, m)| {
                m.joined_period <= p
                    && match m.status {
                        Status::Active => true,
                        Status::Exiting { after_period } => after_period >= p,
                        Status::Left => false,
                    }
                    && !sick.contains(a)
            })
            .map(|(a, m)| (a.clone(), m.level as u128, m.pot))
            .collect();
        donors.sort_by(|x, y| x.0.cmp(&y.0));

        let demand: u128 = recipients.iter().map(|r| r.1).sum();
        let weights: Vec<u128> = donors.iter().map(|d| d.1).collect();
        let shares = apportion(demand, &weights);
        let charges: Vec<u128> = donors.iter().zip(shares.iter()).map(|(d, s)| (*s).min(d.2)).collect();
        let collected: u128 = charges.iter().sum();
        let gift_weights: Vec<u128> = recipients.iter().map(|r| r.1).collect();
        let payouts = apportion(collected, &gift_weights);

        for (d, c) in donors.iter().zip(charges.iter()) {
            if *c > 0 {
                let m = self.members.get_mut(&d.0).unwrap();
                m.pot -= c;
            }
        }
        self.total_pots -= collected;
        let mut lines = Vec::with_capacity(recipients.len());
        for ((a, due), paid) in recipients.iter().zip(payouts.iter()) {
            let m = self.members.get_mut(a).unwrap();
            m.gift_balance += paid;
            let r = self.reports.get_mut(a).unwrap();
            r.gifts_paid += 1;
            lines.push(GiftLine { account_id: a.clone(), due: U128(*due), paid: U128(*paid) });
        }
        self.total_gift_balances += collected;

        // Clean up reports that are finished after this period.
        let done: Vec<AccountId> = self
            .reports
            .iter()
            .filter(|(a, r)| {
                r.gifts_paid >= max_gifts
                    || r.closes_from.map_or(false, |c| c <= p + 1)
                    || !matches!(self.members.get(*a), Some(m) if m.status != Status::Left)
            })
            .map(|(a, _)| a.clone())
            .collect();
        for a in done {
            self.reports.remove(&a);
        }

        let s = Settlement {
            period: p,
            demand: U128(demand),
            collected: U128(collected),
            coverage_bps: coverage_bps(collected, demand),
            donors: donors.len() as u32,
            gifts: lines,
            pots_after: U128(self.total_pots),
            settled_by: env::predecessor_account_id(),
            settled_at_ns: U64(env::block_timestamp()),
        };
        self.settlements.push(s.clone());
        self.next_to_settle = p + 1;
        emit(
            "period_settled",
            json!({"period": p, "demand": U128(demand), "collected": U128(collected),
                   "coverage_bps": s.coverage_bps, "recipients": s.gifts.len()}),
        );
        s
    }

    // ---------- leaving and withdrawals ----------

    /// Start leaving. You still take part in the current period; your pot comes back once
    /// it's settled. A sick member who leaves keeps gifts through the current period.
    pub fn request_exit(&mut self) {
        let me = env::predecessor_account_id();
        self.active_member(&me);
        let period = self.now_period();
        self.members.get_mut(&me).unwrap().status = Status::Exiting { after_period: period };
        if let Some(r) = self.reports.get_mut(&me) {
            let close = period + 1;
            r.closes_from = Some(r.closes_from.map_or(close, |c| c.min(close)));
        }
        emit("exit_requested", json!({"account_id": me, "after_period": period}));
    }

    /// Withdraw gifts you've received, plus your pot if your exit is final.
    /// Returns `true` straight away if your exit is final and there's nothing left to send.
    pub fn withdraw(&mut self) -> PromiseOrValue<bool> {
        let me = env::predecessor_account_id();
        let next = self.next_to_settle;
        let m = self.members.get_mut(&me).unwrap_or_else(|| env::panic_str("Not a member"));
        let gifts = m.gift_balance;
        let mut pot = 0;
        let prev_status = m.status.clone();
        if let Status::Exiting { after_period } = m.status {
            if next > after_period {
                pot = m.pot;
                m.pot = 0;
                m.status = Status::Left;
            }
        }
        let amount = gifts + pot;
        if amount == 0 && m.status == Status::Left {
            // Exit is final and the pot was already empty: just record that they left.
            emit("withdrawn", json!({"account_id": me, "gifts": U128(0), "pot": U128(0)}));
            return PromiseOrValue::Value(true);
        }
        require!(amount > 0, "Nothing to withdraw yet");
        m.gift_balance = 0;
        self.total_gift_balances -= gifts;
        self.total_pots -= pot;
        emit("withdraw_started", json!({"account_id": me, "gifts": U128(gifts), "pot": U128(pot)}));
        ext_ft_core::ext(self.token.clone())
            .with_attached_deposit(NearToken::from_yoctonear(1))
            .with_static_gas(GAS_FT_TRANSFER)
            .ft_transfer(me.clone(), U128(amount), Some("Kring withdrawal".to_string()))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(GAS_CALLBACK)
                    .on_withdraw(me, U128(gifts), U128(pot), prev_status),
            )
            .into()
    }

    #[private]
    pub fn on_withdraw(
        &mut self,
        account_id: AccountId,
        gifts: U128,
        pot: U128,
        prev_status: Status,
        #[callback_result] result: Result<(), PromiseError>,
    ) -> bool {
        require!(
            env::predecessor_account_id() == env::current_account_id(),
            "Only the circle can call this"
        );
        if result.is_ok() {
            emit("withdrawn", json!({"account_id": account_id, "gifts": gifts, "pot": pot}));
            return true;
        }
        // Transfer failed (for example the member isn't registered with the token): put it all back.
        let m = self.members.get_mut(&account_id).unwrap();
        m.gift_balance += gifts.0;
        m.pot += pot.0;
        if pot.0 > 0 {
            m.status = prev_status;
        }
        self.total_gift_balances += gifts.0;
        self.total_pots += pot.0;
        emit("withdraw_failed", json!({"account_id": account_id, "gifts": gifts, "pot": pot}));
        false
    }

    // ---------- governance: member votes only ----------

    /// Needs PROPOSAL_BOND attached. The circle keeps it to pay for storage.
    #[payable]
    pub fn propose(&mut self, kind: ProposalKind) -> u64 {
        let me = env::predecessor_account_id();
        self.active_member(&me);
        require!(
            env::attached_deposit() >= PROPOSAL_BOND,
            "Attach 0.01 NEAR to pay for the proposal's storage"
        );
        match &kind {
            ProposalKind::Admit { account_id, level } => {
                require!(*level >= 1 && *level <= MAX_LEVEL, "level must be 1 to 8");
                require!(*account_id != env::current_account_id(), "The circle can't be a member");
                require!(
                    !matches!(self.members.get(account_id), Some(m) if m.status != Status::Left),
                    "Already a member"
                );
                let in_circle = self.members.values().filter(|m| m.status != Status::Left).count();
                require!(in_circle < MAX_MEMBERS, "The circle is full (50 members)");
            }
            ProposalKind::Expel { account_id } => {
                require!(account_id != &me, "You can't propose expelling yourself");
                require!(
                    matches!(self.members.get(account_id), Some(m) if m.status != Status::Left),
                    "Not a current member"
                );
            }
            ProposalKind::Upgrade { code_hash } => {
                require!(
                    code_hash.len() == 64 && code_hash.chars().all(|c| c.is_ascii_hexdigit()),
                    "code_hash must be 64 hex characters"
                );
            }
        }
        let id = self.proposals.len() as u64;
        self.proposals.push(Proposal {
            id,
            kind: kind.clone(),
            proposer: me.clone(),
            created_ns: U64(env::block_timestamp()),
            approvals: vec![me.clone()],
            vetoed_by: None,
            status: ProposalStatus::Open,
        });
        emit("proposal_created", json!({"id": id, "kind": kind, "proposer": me}));
        id
    }

    fn open_proposal(&self, id: u64) -> Proposal {
        let p = self.proposals.get(id as u32).unwrap_or_else(|| env::panic_str("No such proposal")).clone();
        require!(p.status == ProposalStatus::Open, "Proposal isn't open");
        let expires = p.created_ns.0 + 2 * self.config.proposal_secs as u64 * 1_000_000_000;
        require!(env::block_timestamp() < expires, "Proposal expired");
        p
    }

    pub fn approve(&mut self, id: u64) {
        let me = env::predecessor_account_id();
        self.active_member(&me);
        let p = self.open_proposal(id);
        if let ProposalKind::Expel { account_id } = &p.kind {
            require!(account_id != &me, "You can't vote on your own expulsion");
        }
        require!(!p.approvals.contains(&me), "Already approved");
        self.proposals.get_mut(id as u32).unwrap().approvals.push(me.clone());
        emit("proposal_approved", json!({"id": id, "account_id": me}));
    }

    /// Any one member can veto an admission. That's the rule offline circles use too.
    pub fn veto(&mut self, id: u64) {
        let me = env::predecessor_account_id();
        self.active_member(&me);
        let p = self.open_proposal(id);
        require!(matches!(p.kind, ProposalKind::Admit { .. }), "Only admissions can be vetoed");
        let prop = self.proposals.get_mut(id as u32).unwrap();
        prop.status = ProposalStatus::Vetoed;
        prop.vetoed_by = Some(me.clone());
        emit("proposal_vetoed", json!({"id": id, "account_id": me}));
    }

    /// Counts approvals from members who are active right now.
    fn tally(&self, p: &Proposal) -> (usize, usize) {
        let subject = match &p.kind {
            ProposalKind::Expel { account_id } => Some(account_id.clone()),
            _ => None,
        };
        let voters = self
            .members
            .iter()
            .filter(|(a, m)| m.status == Status::Active && Some((*a).clone()) != subject)
            .count();
        let yes = p
            .approvals
            .iter()
            .filter(|a| self.is_active(a) && Some((*a).clone()) != subject)
            .count();
        (yes, voters)
    }

    /// Anyone can execute a proposal that has passed. Upgrades use `execute_upgrade`.
    pub fn execute(&mut self, id: u64) {
        let p = self.open_proposal(id);
        let (yes, voters) = self.tally(&p);
        let period = self.now_period();
        match p.kind.clone() {
            ProposalKind::Admit { account_id, level } => {
                let veto_ends = p.created_ns.0 + self.config.proposal_secs as u64 * 1_000_000_000;
                require!(env::block_timestamp() >= veto_ends, "Veto window still open");
                require!(yes * 2 > voters, "Needs approval from more than half of active members");
                require!(
                    !matches!(self.members.get(&account_id), Some(m) if m.status != Status::Left),
                    "Already a member"
                );
                let in_circle = self.members.values().filter(|m| m.status != Status::Left).count();
                require!(in_circle < MAX_MEMBERS, "The circle is full (50 members)");
                let carried = self.members.get(&account_id).map_or(0, |m| m.gift_balance);
                self.members.insert(
                    account_id.clone(),
                    Member {
                        level,
                        pot: 0,
                        deposited: 0,
                        joined_period: period,
                        status: Status::Active,
                        gift_balance: carried,
                        expelled: false,
                    },
                );
                emit("member_admitted", json!({"account_id": account_id, "level": level, "period": period}));
            }
            ProposalKind::Expel { account_id } => {
                require!(yes * 2 > voters, "Needs approval from more than half of active members");
                let next = self.next_to_settle;
                let m = self.members.get_mut(&account_id).unwrap_or_else(|| env::panic_str("Not a member"));
                require!(m.status != Status::Left, "Not a current member");
                m.expelled = true;
                if m.status == Status::Active {
                    m.status = Status::Exiting { after_period: period };
                }
                // A fake claim stops now, even for periods not yet settled.
                if let Some(r) = self.reports.get_mut(&account_id) {
                    r.closes_from = Some(next);
                }
                emit("member_expelled", json!({"account_id": account_id}));
            }
            ProposalKind::Upgrade { .. } => env::panic_str("Use execute_upgrade with the code"),
        }
        self.proposals.get_mut(id as u32).unwrap().status = ProposalStatus::Executed;
    }

    /// Raw input: 8-byte little-endian proposal id, then the wasm. Needs two thirds of
    /// active members, and the wasm's sha256 must match the proposal.
    pub fn execute_upgrade(&mut self) -> Promise {
        let input = env::input().unwrap_or_default();
        require!(input.len() > 8, "Input is proposal id (8 bytes LE) followed by the wasm");
        let mut idb = [0u8; 8];
        idb.copy_from_slice(&input[0..8]);
        let id = u64::from_le_bytes(idb);
        let code = &input[8..];
        let p = self.open_proposal(id);
        let want = match &p.kind {
            ProposalKind::Upgrade { code_hash } => code_hash.to_lowercase(),
            _ => env::panic_str("Not an upgrade proposal"),
        };
        let got: String = env::sha256(code).iter().map(|b| format!("{:02x}", b)).collect();
        require!(got == want, "Code doesn't match the approved hash");
        let (yes, voters) = self.tally(&p);
        require!(yes * 3 >= voters * 2, "Upgrades need two thirds of active members");
        self.proposals.get_mut(id as u32).unwrap().status = ProposalStatus::Executed;
        emit("upgraded", json!({"id": id, "code_hash": want}));
        Promise::new(env::current_account_id()).deploy_contract(code.to_vec())
    }

    // ---------- views ----------

    pub fn get_circle(&self) -> CircleView {
        let now = env::block_timestamp();
        let cur = self.period_at(now);
        CircleView {
            token: self.token.clone(),
            config: self.config.clone(),
            genesis_ns: U64(self.genesis_ns),
            now_ns: U64(now),
            current_period: U64(cur),
            next_to_settle: U64(self.next_to_settle),
            settle_ready: cur > self.next_to_settle,
            active_members: self.members.values().filter(|m| m.status == Status::Active).count() as u32,
            exiting_members: self
                .members
                .values()
                .filter(|m| matches!(m.status, Status::Exiting { .. }))
                .count() as u32,
            open_reports: self.reports.len(),
            total_pots: U128(self.total_pots),
            total_gift_balances: U128(self.total_gift_balances),
            last_coverage_bps: if self.settlements.is_empty() {
                None
            } else {
                Some(self.settlements.get(self.settlements.len() - 1).unwrap().coverage_bps)
            },
            proposals: self.proposals.len() as u64,
        }
    }

    fn member_view(&self, a: &AccountId, m: &Member) -> MemberView {
        let req = self.required_to_date(m);
        MemberView {
            account_id: a.clone(),
            level: m.level,
            pot: U128(m.pot),
            deposited: U128(m.deposited),
            required_deposit_to_date: U128(req),
            good_standing: m.deposited >= req,
            joined_period: U64(m.joined_period),
            status: m.status.clone(),
            gift_balance: U128(m.gift_balance),
            gift_per_period: U128(self.gift_of(m.level)),
            deposit_per_period: U128(self.deposit_of(m.level)),
            expelled: m.expelled,
        }
    }

    pub fn get_member(&self, account_id: AccountId) -> Option<MemberView> {
        self.members.get(&account_id).map(|m| self.member_view(&account_id, m))
    }

    pub fn get_members(&self) -> Vec<MemberView> {
        self.members.iter().map(|(a, m)| self.member_view(a, m)).collect()
    }

    fn report_view(&self, a: &AccountId, r: &Report) -> ReportView {
        ReportView {
            account_id: a.clone(),
            opened_period: U64(r.opened_period),
            opened_ns: U64(r.opened_ns),
            vouch_deadline_ns: U64(r.vouch_deadline_ns),
            vouchers: r.vouchers.clone(),
            vouched: r.vouched.clone(),
            confirmed_period: r.confirmed_period.map(U64),
            first_gift_period: U64(r.opened_period + self.config.waiting_periods as u64),
            gifts_paid: r.gifts_paid,
            closes_from: r.closes_from.map(U64),
        }
    }

    pub fn get_report(&self, account_id: AccountId) -> Option<ReportView> {
        self.reports.get(&account_id).map(|r| self.report_view(&account_id, r))
    }

    pub fn get_reports(&self) -> Vec<ReportView> {
        self.reports.iter().map(|(a, r)| self.report_view(a, r)).collect()
    }

    pub fn get_settlements(&self, from: Option<u64>, limit: Option<u64>) -> Vec<Settlement> {
        let from = from.unwrap_or(0);
        let limit = limit.unwrap_or(24).min(100);
        (from..(from + limit).min(self.settlements.len() as u64))
            .map(|i| self.settlements.get(i as u32).unwrap().clone())
            .collect()
    }

    pub fn get_proposals(&self, from: Option<u64>, limit: Option<u64>) -> Vec<Proposal> {
        let from = from.unwrap_or(0);
        let limit = limit.unwrap_or(50).min(100);
        (from..(from + limit).min(self.proposals.len() as u64))
            .map(|i| self.proposals.get(i as u32).unwrap().clone())
            .collect()
    }

    /// The bookkeeping identity the token balance must match: pots plus unwithdrawn gifts.
    pub fn get_held(&self) -> U128 {
        U128(self.total_pots + self.total_gift_balances)
    }
}

#[near]
impl FungibleTokenReceiver for Circle {
    /// Deposit into your own pot with ft_transfer_call. Anything that isn't a deposit from an
    /// active member is handed straight back.
    fn ft_on_transfer(&mut self, sender_id: AccountId, amount: U128, msg: String) -> PromiseOrValue<U128> {
        require!(env::predecessor_account_id() == self.token, "Only the circle's token is accepted");
        let _ = msg;
        let active = self.is_active(&sender_id);
        if !active || amount.0 == 0 {
            emit("deposit_refused", json!({"account_id": sender_id, "amount": amount}));
            return PromiseOrValue::Value(amount);
        }
        let m = self.members.get_mut(&sender_id).unwrap();
        m.pot += amount.0;
        m.deposited += amount.0;
        self.total_pots += amount.0;
        emit("deposited", json!({"account_id": sender_id, "amount": amount}));
        PromiseOrValue::Value(U128(0))
    }
}
