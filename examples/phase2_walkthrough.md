# Phase 2 Demo Walkthrough

This walkthrough validates Phase 2 features with reproducible commands and expected output highlights.

## Fixtures

- `examples/software_license.k` - baseline contract with business-day terms and remedy cascade flow
- `examples/software_license_v1.k` - baseline contract for run/eval/simulate validation
- `examples/software_license_v2.k` - revised contract for semantic diff validation

Coverage map:

- business-day behavior: `software_license.k`, `software_license_v1.k`
- simulation: `software_license_v1.k`
- diff + warnings: `software_license_v1.k` vs `software_license_v2.k`
- cascade tracing: `software_license.k`

## 1) Contract summary (`run`)

Command:

```bash
cargo +stable run -- run examples/software_license_v1.k
```

Expected highlights:

- `Parties (2): licensee, licensor`
- `Terms (3): CurePeriod, DeliveryPeriod, PaymentWindow`
- `Remedies (1): CureOrTerminate`
- `Phases (2): CureOrTerminate.Cure, CureOrTerminate.Terminate`

## 2) State evaluation (`eval`)

Command:

```bash
cargo +stable run -- eval examples/software_license_v1.k --trigger Effective=2026-03-01 --trigger Delivery=2026-03-20 --trigger AcceptanceNotice=2026-03-25 --at 2026-05-20
```

Expected highlights:

- `DeliverSoftware: BREACHED (...)`
- `CureOrTerminate.Cure: BREACHED (...)`
- `CureOrTerminate.Terminate: ACTIVE`
- `PayLicenseFee: PENDING (blocked - condition not met)`

## 3) Non-destructive simulation (`simulate`)

Command:

```bash
cargo +stable run -- simulate examples/software_license_v1.k --trigger Effective=2026-03-01 --trigger Delivery=2026-03-20 --trigger AcceptanceNotice=2026-03-25 --at 2026-05-20
```

Expected highlights:

- Output contains two labeled sections:
  - `Canonical state:`
  - `Simulated state:`
- Canonical section remains fully pending.
- Simulated section shows breach progression:
  - `DeliverSoftware: BREACHED (...)`
  - `CureOrTerminate.Cure: BREACHED (...)`
  - `CureOrTerminate.Terminate: ACTIVE`

## 4) Contract diff + risk warnings (`diff`)

Command:

```bash
cargo +stable run -- diff examples/software_license_v1.k examples/software_license_v2.k
```

Expected highlights:

- `REMOVED: TERM CurePeriod`
- `CHANGED: TERM DeliveryPeriod — duration.amount: 30 -> 45`
- `ADDED: OBLIGATION AuditRights`
- `REMOVED: REMEDY CureOrTerminate`
- `REMOVED: PHASE CureOrTerminate.Cure`
- `REMOVED: PHASE CureOrTerminate.Terminate`
- `WARNING: breach of 'DeliverSoftware' now has no remedy`

## Optional: Breach cascade tracing (`cascade`)

Command:

```bash
cargo +stable run -- cascade examples/software_license.k DeliverSoftware
```

Expected highlights:

- Header: `Cascade from 'DeliverSoftware':`
- One or more links of the form:
  - `- <source> -> <target> (<reason>)`

## 5) Quality gates (Block 10 closeout)

Run from `kontra/`:

```bash
cargo +stable clippy --all-targets --all-features -- -D warnings
cargo +stable test
cargo +stable run -- run examples/software_license_v1.k
cargo +stable run -- eval examples/software_license_v1.k --trigger Effective=2026-03-01 --trigger Delivery=2026-03-20 --trigger AcceptanceNotice=2026-03-25 --at 2026-05-20
cargo +stable run -- simulate examples/software_license_v1.k --trigger Effective=2026-03-01 --trigger Delivery=2026-03-20 --trigger AcceptanceNotice=2026-03-25 --at 2026-05-20
cargo +stable run -- diff examples/software_license_v1.k examples/software_license_v2.k
```

Pass criteria:

- clippy exits 0 with no warnings allowed
- full test suite exits 0
- each e2e command exits 0 and includes expected highlight lines from sections 1-4

## 6) Verification evidence (2026-02-17)

- `cargo +stable clippy --all-targets --all-features -- -D warnings`: PASS
- `cargo +stable test`: PASS (`109 passed; 0 failed`)
- `run`: PASS (printed Parties/Events/Terms/Obligations/Remedies/Phases summary)
- `eval`: PASS (showed `DeliverSoftware: BREACHED`, `CureOrTerminate.Cure: BREACHED`, `CureOrTerminate.Terminate: ACTIVE`)
- `simulate`: PASS (printed both `Canonical state:` and `Simulated state:` sections)
- `diff`: PASS (showed changed term, added obligation, removed remedy/phases, and warning lines)
- `cascade` (optional): PASS (printed deterministic link chain from `DeliverSoftware`)
