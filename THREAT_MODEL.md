# Kring threat model

Kring is a sick-pay gift circle for freelancers, running as a NEAR testnet prototype. 3 to 50 members
each park a fixed monthly deposit in their own pot. When a member is too sick to work, the healthy
members' pots send a monthly gift after a one-month waiting period, for at most 24 months. Money stays
in each member's own pot until it's gifted, and leaving gives back whatever is left.

The shape comes from Dutch gift circles (Broodfondsen, or schenkkringen), which work this way offline
today. Kring takes only the idea, not any name or branding. The chain replaces the bank account, the
board and the admin. It doesn't replace the trust: circles are invite-only, and members still know
each other.

Every row below has at least one unit test in `contracts/circle/src/tests.rs`. The sandbox run in
`e2e/sandbox-demo.mjs` repeats the most important ones against a real NEAR node.

## Assets

- Member pots (test token tUSD, NEP-141, held by the circle and ledgered per member)
- Gifts owed but not yet withdrawn (`gift_balance`)
- The right to receive gifts (a sick report two drawn peers vouched for)
- Membership itself (invite-only, majority vote, any member can veto an admission)
- The contract code (no single admin; any change needs a member vote)

## Attacks and controls

| # | Attacker | Attack | Control | Tests |
|---|----------|--------|---------|-------|
| 1 | Whoever deployed the circle | Keeps a key and drains the pots later | Deploy, init and delete the only key in one atomic transaction. `scripts/prove-keyless.sh` reads the key list (zero) and the code hash from public RPC. `new` only runs when the circle calls itself. | `init_only_by_the_circle_account_itself`; e2e; `prove-keyless.sh` |
| 2 | Any member, or the deployer | Swaps in new code | `execute_upgrade` only deploys code whose sha256 matches a proposal that at least two thirds of active members approved. | `upgrade_needs_two_thirds`, `upgrade_rejects_different_code`, `upgrade_passes_with_two_thirds_and_matching_code` |
| 3 | Member | Fakes being sick | Two vouchers drawn by the contract must both vouch within the window. One-month waiting period. Gift is fixed by the level the member pays for. A majority can expel, which stops gifts from the next unsettled period. | `unvouched_report_gets_nothing`, `only_drawn_vouchers_can_vouch`, `one_voucher_cannot_count_twice`, `vouch_after_deadline_is_rejected`, `no_gift_in_the_waiting_period`, `expelling_a_fake_claimant_stops_gifts_at_once`, `cannot_vote_on_own_expulsion`, `expulsion_needs_a_majority` |
| 4 | Member | Joins already sick and claims at once | No report during probation (6 periods suggested for real circles, 1 to 2 in the demos), and that applies to admitted members too. | `new_member_cannot_report_during_probation`, `admitted_member_has_probation_too` |
| 5 | Two colluding members | Vouch for each other | The claimant can't pick vouchers: the contract draws them from `env::random_seed`. Nobody vouches for themselves, and the same voucher can't be drawn for the same claimant again for 12 periods. A redraw only replaces vouchers who didn't vouch and never repeats anyone. **Limitation:** block producers can bias the seed. Invite-only membership is the main defense here, not the draw. | `claimant_is_never_their_own_voucher_and_draw_follows_seed`, `same_voucher_cannot_vouch_for_same_claimant_within_cooldown`, `cooldown_ends_after_twelve_periods`, `redraw_replaces_only_missing_vouchers_and_never_repeats`, `redraw_waits_for_the_window` |
| 6 | Member | Leaves the moment a friend gets sick | An exit only takes effect after the current period settles, and the leaver still donates for it. The pot stays locked until then. | `leaver_still_pays_the_current_period`, `pot_is_locked_until_the_exit_period_settles`, `exit_returns_the_pot_after_settlement`, `leaver_with_an_empty_pot_can_still_leave` |
| 7 | Member | Stops paying in, then reports sick | A report needs deposits up to date. Sick members don't donate while they receive. | `member_behind_on_deposits_cannot_report`, `sick_member_does_not_donate_to_others` |
| 8 | Bad luck (too many sick at once) | Pots run dry and early claimants take everything | Each donor gives in proportion to their level, capped at their pot. If pots can't cover the month, every gift scales down by the same ratio, and the coverage ratio is stored on chain. Gifts stop after 24 periods. | `gifts_scale_down_evenly_when_pots_run_low`, `full_coverage_pays_exactly_the_gift_and_splits_by_level`, `gifts_stop_after_24_periods`, `recovery_stops_gifts_from_that_period`; `sim/SIMULATION.md` |
| 9 | Anyone | Settles a period twice, or early, or out of order | `settle()` is permissionless but strictly sequential, only for finished periods, and each period settles once. | `settle_twice_in_one_period_fails`, `current_period_cannot_be_settled`, `settle_is_permissionless_and_sequential` |
| 10 | Anyone | Spams reports | Only active members can report, one open report each. | `outsider_cannot_report`, `one_open_report_per_member`, `non_member_cannot_vouch` |
| 11 | Anyone | Pays in with a fake token to inflate a pot | `ft_on_transfer` only accepts calls from the configured token contract. Deposits from non-members or leavers are handed back. | `fake_token_deposit_is_rejected`, `outsider_deposit_is_refunded`, `exiting_member_deposit_is_refunded` |
| 12 | Anyone | Breaks a withdrawal to lose or double money | Pull pattern: balances are zeroed before the transfer, and the private callback puts everything back (status included) if the transfer fails. | `failed_withdrawal_puts_everything_back`, `failed_exit_withdrawal_restores_status_and_pot`, `withdraw_callback_is_private` |
| 13 | A clique | Pushes in an unwanted member | Admission needs a majority of active members after a veto window, and one veto kills it. Proposals expire. Only admissions can be vetoed, so a lone member can't block an expulsion of themselves or anyone else. | `one_veto_blocks_admission`, `admission_waits_for_the_veto_window`, `admission_by_majority_after_window`, `proposals_expire`, `veto_only_applies_to_admissions`, `outsider_cannot_propose` |
| 14 | Rounding | Tiny amounts appear or vanish over time | Largest-remainder apportionment, so parts always sum exactly. Books are checked against the real token balance. | `math::*`, `books_balance_through_a_busy_year`; e2e final check |
| 15 | Member | Spams proposals so the circle's own NEAR runs out paying for storage, which would block every write | Every proposal carries a 0.01 NEAR bond that the circle keeps (about 1 KB of storage). Anyone can also top the circle up with a plain transfer. Added in v2 by a member-voted upgrade. | `proposal_without_bond_is_rejected`; e2e |

## Known limits

- The random draw can be biased by block producers. With invite-only circles of people who know each other, we think that's acceptable for a prototype, but it isn't a cryptographic guarantee.
- Members need a NEAR wallet today and pay their own (tiny) gas. Sponsored gas is the next step. The meta-transaction relayer was cut from this version on purpose.
- The circle's NEAR balance pays for its own storage. Settlements are kept forever (one per period), which is nothing at monthly periods but adds up with the 3-minute demo periods. Anyone can top the circle up with a plain NEAR transfer.
- The demo wasm is a non-reproducible build. The proof script checks that the code on chain matches `res/kring_circle.wasm` in this repo, not that you'd get the same bytes from your own build.
- Not insurance. Nothing is promised, there's no third party, and gifts can scale down. Testnet prototype with test tokens; any real deployment needs legal review in each jurisdiction.
