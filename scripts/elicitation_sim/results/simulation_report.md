# Intent Elicitation Simulation & Bisection Benchmark

| Brief ID | Title | Turns | Macro Resolved Turn | Axis Coverage | Bisection Rate | Neutrality Score | Status |
| :--- | :--- | :---: | :---: | :---: | :---: | :---: | :---: |
| **B01** | Community Repair Cafe | 6 | Turn 2 | 5/5 (100.0%) | 100% | 100% | **PASSED** |
| **B02** | Rural Walk-in Clinic Triage | 6 | Turn 3 | 5/5 (100.0%) | 100% | 100% | **PASSED** |
| **B03** | Refrigerated Cold-Chain Freight | 6 | Turn 3 | 5/5 (100.0%) | 100% | 100% | **PASSED** |

### Detailed Axis Breakdown

#### B01: Community Repair Cafe
- **Macro Envelope Resolution:** Turn 2
  - `agency`: ✓ (anchors: who, fixer, responsibility, sign-off, volunteer)
  - `preconditions`: ✓ (anchors: must, require, safety, rule)
  - `exceptions`: ✓ (anchors: if, conflict)
  - `cadence`: ✓ (anchors: saturday, temporal)
  - `accountability`: ✓ (anchors: record, sign, track)
- **Neutrality:** Clean (0 violations)

#### B02: Rural Walk-in Clinic Triage
- **Macro Envelope Resolution:** Turn 3
  - `agency`: ✓ (anchors: nurse, authority, responsibility)
  - `preconditions`: ✓ (anchors: require, rule)
  - `exceptions`: ✓ (anchors: if, delay, emergency, override)
  - `cadence`: ✓ (anchors: minute, hour, temporal)
  - `accountability`: ✓ (anchors: sign, audit, document, evidence)
- **Neutrality:** Clean (0 violations)

#### B03: Refrigerated Cold-Chain Freight
- **Macro Envelope Resolution:** Turn 3
  - `agency`: ✓ (anchors: driver, manager, authority)
  - `preconditions`: ✓ (anchors: must, require, strictly)
  - `exceptions`: ✓ (anchors: if, when, delay, dispute, excursion)
  - `cadence`: ✓ (anchors: minute, schedule, shift, transit, temporal)
  - `accountability`: ✓ (anchors: log, sign, telemetry, seal, serial)
- **Neutrality:** Clean (0 violations)
