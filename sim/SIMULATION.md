# Kring simulation (seed 2026)

**This is a simulation, not data.** It runs the real circle contract code in near-sdk's mocked blockchain. The sickness patterns below are assumptions picked to stress the circle, not statistics.

Setup: 20 members, five on each level 1 to 4, gift 500 to 2000 tUSD a month, deposit 1/20 of the gift every month (the ratio Broodfonds uses), one-month waiting period, gifts for at most 24 months, monthly settlement, pots start empty. Sick members keep paying their deposit. Vouchers always vouch.

Coverage is the share of the month's gifts the healthy members' pots could pay. Below 100%, every gift is scaled down by the same factor rather than anyone's pot going negative.

Reproduce: `cargo run --example simulate -- 2026` from `contracts/circle`.

| Scenario | Assumption | Spells | Gifts due (tUSD) | Gifts paid (tUSD) | Months below 100% | Lowest coverage | Empty pots at worst | Pots left (tUSD) |
|---|---|---|---|---|---|---|---|---|
| A. Baseline | 1% of healthy members start a long spell each month; spells average 4 months. | 5 | 17000 | 17000 | 0 | 100.0% | 0 | 13000 |
| B. Two bad years | 3% a month, spells average 6 months. Roughly three times the baseline load. | 11 | 75500 | 27386 | 19 | 14.8% | 15 | 2614 |
| C. Shock | Baseline, plus four members (one per level) fall sick in month 3 and stay sick. | 8 | 113000 | 24235 | 20 | 12.8% | 16 | 5765 |

"Spells" counts new sickness spells in the 24 months. "Sick" in the tables below is the count at the end of each month's activity, including members still in their waiting month.

## A. Baseline

1% of healthy members start a long spell each month; spells average 4 months.

| Month | Sick | Paid | Due (tUSD) | Paid (tUSD) | Coverage | Empty pots | Pots after (tUSD) |
|---|---|---|---|---|---|---|---|
| 0 | 0 | 0 | 0 | 0 | 100.0% | 0 | 1250 |
| 1 | 0 | 0 | 0 | 0 | 100.0% | 0 | 2500 |
| 2 | 0 | 0 | 0 | 0 | 100.0% | 0 | 3750 |
| 3 | 0 | 0 | 0 | 0 | 100.0% | 0 | 5000 |
| 4 | 0 | 0 | 0 | 0 | 100.0% | 0 | 6250 |
| 5 | 0 | 0 | 0 | 0 | 100.0% | 0 | 7500 |
| 6 | 0 | 0 | 0 | 0 | 100.0% | 0 | 8750 |
| 7 | 1 | 0 | 0 | 0 | 100.0% | 0 | 10000 |
| 8 | 0 | 0 | 0 | 0 | 100.0% | 0 | 11250 |
| 9 | 3 | 0 | 0 | 0 | 100.0% | 0 | 12500 |
| 10 | 2 | 2 | 3500 | 3500 | 100.0% | 0 | 10250 |
| 11 | 2 | 2 | 3500 | 3500 | 100.0% | 0 | 8000 |
| 12 | 2 | 2 | 3500 | 3500 | 100.0% | 0 | 5750 |
| 13 | 1 | 1 | 2000 | 2000 | 100.0% | 0 | 5000 |
| 14 | 0 | 0 | 0 | 0 | 100.0% | 0 | 6250 |
| 15 | 0 | 0 | 0 | 0 | 100.0% | 0 | 7500 |
| 16 | 0 | 0 | 0 | 0 | 100.0% | 0 | 8750 |
| 17 | 0 | 0 | 0 | 0 | 100.0% | 0 | 10000 |
| 18 | 1 | 0 | 0 | 0 | 100.0% | 0 | 11250 |
| 19 | 1 | 1 | 1500 | 1500 | 100.0% | 0 | 11000 |
| 20 | 1 | 1 | 1500 | 1500 | 100.0% | 0 | 10750 |
| 21 | 1 | 1 | 1500 | 1500 | 100.0% | 0 | 10500 |
| 22 | 0 | 0 | 0 | 0 | 100.0% | 0 | 11750 |
| 23 | 0 | 0 | 0 | 0 | 100.0% | 0 | 13000 |

## B. Two bad years

3% a month, spells average 6 months. Roughly three times the baseline load.

| Month | Sick | Paid | Due (tUSD) | Paid (tUSD) | Coverage | Empty pots | Pots after (tUSD) |
|---|---|---|---|---|---|---|---|
| 0 | 0 | 0 | 0 | 0 | 100.0% | 0 | 1250 |
| 1 | 2 | 0 | 0 | 0 | 100.0% | 0 | 2500 |
| 2 | 1 | 1 | 1500 | 1500 | 100.0% | 0 | 2250 |
| 3 | 2 | 1 | 1500 | 1500 | 100.0% | 0 | 2000 |
| 4 | 3 | 2 | 2500 | 2500 | 100.0% | 0 | 750 |
| 5 | 5 | 3 | 3000 | 1145 | 38.2% | 15 | 855 |
| 6 | 5 | 5 | 4500 | 1025 | 22.8% | 15 | 1080 |
| 7 | 2 | 2 | 1500 | 1216 | 81.1% | 15 | 1113 |
| 8 | 2 | 2 | 1500 | 1216 | 81.1% | 15 | 1147 |
| 9 | 2 | 2 | 1500 | 1216 | 81.1% | 15 | 1180 |
| 10 | 2 | 2 | 1500 | 1216 | 81.1% | 15 | 1214 |
| 11 | 3 | 2 | 1500 | 1155 | 77.0% | 14 | 1309 |
| 12 | 4 | 2 | 2500 | 1237 | 49.5% | 14 | 1323 |
| 13 | 4 | 4 | 4000 | 1331 | 33.3% | 14 | 1242 |
| 14 | 5 | 4 | 4000 | 1198 | 29.9% | 14 | 1294 |
| 15 | 5 | 5 | 4500 | 1082 | 24.1% | 15 | 1461 |
| 16 | 5 | 5 | 4500 | 1025 | 22.8% | 15 | 1686 |
| 17 | 5 | 5 | 4500 | 1025 | 22.8% | 15 | 1911 |
| 18 | 6 | 5 | 4500 | 925 | 20.6% | 14 | 2236 |
| 19 | 5 | 5 | 5500 | 1207 | 21.9% | 14 | 2279 |
| 20 | 5 | 4 | 5000 | 1153 | 23.1% | 14 | 2376 |
| 21 | 5 | 5 | 6500 | 965 | 14.8% | 15 | 2661 |
| 22 | 4 | 3 | 4500 | 1262 | 28.1% | 15 | 2649 |
| 23 | 5 | 4 | 5000 | 1285 | 25.7% | 14 | 2614 |

## C. Shock

Baseline, plus four members (one per level) fall sick in month 3 and stay sick.

| Month | Sick | Paid | Due (tUSD) | Paid (tUSD) | Coverage | Empty pots | Pots after (tUSD) |
|---|---|---|---|---|---|---|---|
| 0 | 0 | 0 | 0 | 0 | 100.0% | 0 | 1250 |
| 1 | 1 | 0 | 0 | 0 | 100.0% | 0 | 2500 |
| 2 | 1 | 1 | 2000 | 2000 | 100.0% | 0 | 1750 |
| 3 | 5 | 1 | 2000 | 2000 | 100.0% | 0 | 1000 |
| 4 | 5 | 5 | 7000 | 935 | 13.3% | 15 | 1315 |
| 5 | 5 | 5 | 7000 | 900 | 12.8% | 15 | 1665 |
| 6 | 4 | 4 | 5000 | 1400 | 28.0% | 15 | 1515 |
| 7 | 4 | 4 | 5000 | 1200 | 24.0% | 16 | 1565 |
| 8 | 4 | 4 | 5000 | 1000 | 20.0% | 16 | 1815 |
| 9 | 4 | 4 | 5000 | 1000 | 20.0% | 16 | 2065 |
| 10 | 4 | 4 | 5000 | 1000 | 20.0% | 16 | 2315 |
| 11 | 5 | 4 | 5000 | 975 | 19.5% | 15 | 2590 |
| 12 | 5 | 5 | 5500 | 975 | 17.7% | 15 | 2865 |
| 13 | 5 | 5 | 5500 | 975 | 17.7% | 15 | 3140 |
| 14 | 4 | 4 | 5000 | 1075 | 21.5% | 16 | 3315 |
| 15 | 4 | 4 | 5000 | 1000 | 20.0% | 16 | 3565 |
| 16 | 4 | 4 | 5000 | 1000 | 20.0% | 16 | 3815 |
| 17 | 4 | 4 | 5000 | 1000 | 20.0% | 16 | 4065 |
| 18 | 5 | 4 | 5000 | 975 | 19.5% | 15 | 4340 |
| 19 | 5 | 5 | 5500 | 975 | 17.7% | 15 | 4615 |
| 20 | 6 | 5 | 5500 | 925 | 16.8% | 14 | 4940 |
| 21 | 5 | 5 | 6000 | 1025 | 17.1% | 15 | 5165 |
| 22 | 5 | 5 | 6000 | 950 | 15.8% | 15 | 5465 |
| 23 | 5 | 5 | 6000 | 950 | 15.8% | 15 | 5765 |
