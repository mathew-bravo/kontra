# kontra

**Contracts as code.** A DSL and runtime engine that turns legal agreements into executable specifications you can compile, evaluate, simulate, diff, and query.

Built for the 2026 hackathon. ~6,400 lines of Rust. 109 tests. No parser generators — hand-rolled scanner and compiler following the [Crafting Interpreters](https://craftinginterpreters.com/) architecture.

---

## The problem

Contracts are the atomic unit of legal work. Every deal, every engagement, every relationship between parties comes down to a contract. But contracts are written in natural language — ambiguous by nature, impossible to run, and requiring expensive human interpretation to answer even basic questions like *"what happens if the vendor is 15 days late?"*

You can't test a contract. You can't simulate what happens if a deadline slips. You can't diff two versions and see which obligations changed, which remedies disappeared, or where new risk was introduced.

**kontra** fixes that.

## What it does

kontra is a domain-specific language where obligations, conditions, deadlines, remedies, and termination clauses are formal constructs that a machine can evaluate. The contract is still a contract — it expresses the agreement between parties — but now it can also be:

- **Compiled** and structurally validated
- **Evaluated** at any point in time to see which obligations are active, satisfied, or breached
- **Simulated** non-destructively to test "what if" scenarios before commitments are made
- **Diffed** semantically to detect risky changes between contract versions
- **Traced** to explain downstream breach cascades with deterministic output

Think of it as Terraform for legal agreements: declarative, version-controlled, and executable.

---

## Quick start

```bash
git clone https://github.com/mathew-bravo/kontra.git
cd kontra
cargo build
cargo run -- run examples/software_license.k
```

---

## The DSL

Here's a real contract written in `.k` syntax — a software license agreement between two parties:

```
contract SoftwareLicense {
  parties {
    licensor: "Acme Corp"
    licensee: "Beta Inc."
  }

  event Effective = date("2026-03-01")
  event Delivery = triggered_by(licensor)
  event AcceptanceNotice = triggered_by(licensee)

  term DeliveryPeriod = 30 business_days from Effective
  term AcceptancePeriod = 14 calendar_days from Delivery
  term CurePeriod = 10 business_days from breach_of(DeliverSoftware)

  obligation DeliverSoftware {
    party: licensor
    action: "Deliver software per Schedule A"
    due: DeliveryPeriod
    condition: after(Effective)
  }

  obligation PayLicenseFee {
    party: licensee
    action: "Pay $50,000 license fee"
    due: 15 calendar_days from AcceptanceNotice
    condition: after(Delivery) and occurred(AcceptanceNotice)
  }

  remedy LateFee on breach_of(PayLicenseFee) {
    party: licensee
    action: "Pay 1.5% monthly interest on outstanding amount"
    due: AcceptancePeriod
    condition: occurred(AcceptanceNotice)
  }

  remedy CureOrTerminate on breach_of(DeliverSoftware) {
    phase Cure {
      party: licensor
      action: "Deliver software within cure period"
      due: CurePeriod
      condition: after(Effective)
    }
    phase Terminate on breach_of(Cure) {
      action: "Licensor refunds any amounts paid"
    }
  }
}
```

Parties, events, terms, obligations, remedies, phases — all declared, all executable. Comments use `--`.

---

## CLI commands

kontra ships six commands. Every example below uses real output from the working binary.

### `run` — compile and summarize

Parse a `.k` file, compile it through the full pipeline, and print the contract's structural model.

```bash
kontra run examples/software_license.k
```

```
Parties (2): licensee, licensor
Events (3): AcceptanceNotice, Delivery, Effective
Terms (3): AcceptancePeriod, CurePeriod, DeliveryPeriod
Obligations (2): DeliverSoftware, PayLicenseFee
Remedies (2): CureOrTerminate, LateFee
Phases (2): CureOrTerminate.Cure, CureOrTerminate.Terminate
```

This proves the contract is parsed and executed semantics, not static text.

### `eval` — evaluate contract state at a point in time

Trigger events with concrete dates and see what the contract looks like on a given day.

**Happy path** — delivery happened, acceptance was given, obligations are active:

```bash
kontra eval examples/software_license.k \
  --trigger Effective=2026-03-01 \
  --trigger Delivery=2026-03-20 \
  --trigger AcceptanceNotice=2026-03-25 \
  --at 2026-03-26
```

```
CureOrTerminate: PENDING (blocked - condition not met)
CureOrTerminate.Cure: PENDING (blocked - condition not met)
CureOrTerminate.Terminate: PENDING (blocked - condition not met)
DeliverSoftware: ACTIVE (due 2026-04-10)
LateFee: PENDING (blocked - condition not met)
PayLicenseFee: ACTIVE (due 2026-04-09)
```

**Breach progression** — no delivery, deadline passes, cure period expires, termination activates:

```bash
kontra eval examples/software_license.k \
  --trigger Effective=2026-03-01 \
  --at 2026-05-20
```

```
CureOrTerminate: ACTIVE
CureOrTerminate.Cure: BREACHED (due 2026-04-24, OVERDUE by 26 days)
CureOrTerminate.Terminate: ACTIVE
DeliverSoftware: BREACHED (due 2026-04-10, OVERDUE by 40 days)
LateFee: PENDING (blocked - condition not met)
PayLicenseFee: PENDING (blocked - condition not met)
```

The engine captures downstream legal mechanics — breach cascades through remedy phases automatically.

### `simulate` — non-destructive what-if scenarios

Fork the contract state, apply hypothetical events, and compare baseline vs. simulated outcomes side-by-side. The canonical state is never mutated.

```bash
kontra simulate examples/software_license_v1.k \
  --trigger Effective=2026-03-01 \
  --trigger Delivery=2026-03-20 \
  --trigger AcceptanceNotice=2026-03-25 \
  --at 2026-05-20
```

```
Canonical state:
CureOrTerminate: PENDING (blocked - condition not met)
CureOrTerminate.Cure: PENDING (blocked - condition not met)
CureOrTerminate.Terminate: PENDING (blocked - condition not met)
DeliverSoftware: PENDING (blocked - condition not met)
PayLicenseFee: PENDING (blocked - condition not met)

Simulated state:
CureOrTerminate: ACTIVE
CureOrTerminate.Cure: BREACHED (due 2026-04-24, OVERDUE by 26 days)
CureOrTerminate.Terminate: ACTIVE
DeliverSoftware: BREACHED (due 2026-04-10, OVERDUE by 40 days)
PayLicenseFee: PENDING (blocked - condition not met)
```

Teams can test operational scenarios before legal or business commitments are made.

### `diff` — semantic contract comparison with risk warnings

Compare two contract versions structurally — not as plain text, but as semantic deltas with automated risk detection.

```bash
kontra diff examples/software_license_v1.k examples/software_license_v2.k
```

```
REMOVED: TERM CurePeriod
CHANGED: TERM DeliveryPeriod — duration.amount: 30 -> 45
ADDED: OBLIGATION AuditRights
REMOVED: REMEDY CureOrTerminate
REMOVED: PHASE CureOrTerminate.Cure
REMOVED: PHASE CureOrTerminate.Terminate

WARNING: breach of 'DeliverSoftware' now has no remedy
WARNING: removed remedy phase 'CureOrTerminate.Cure'
WARNING: removed remedy phase 'CureOrTerminate.Terminate'
```

The delivery window got extended from 30 to 45 business days. An audit right was added. But the entire cure-or-terminate remedy was removed — meaning a breach of `DeliverSoftware` now has no recourse. That's the kind of thing that gets missed in redlines.

### `cascade` — trace downstream breach impact

Given a breached obligation, deterministically trace every downstream effect — which remedies fire, which phases activate, which due dates shift.

```bash
kontra cascade examples/software_license.k DeliverSoftware
```

```
Cascade from 'DeliverSoftware':
- CureOrTerminate.Cure -> CureOrTerminate.Terminate (phase breach trigger)
- DeliverSoftware -> CureOrTerminate (remedy breach target)
- DeliverSoftware -> CureOrTerminate.Cure (due date anchored by term 'CurePeriod')
- DeliverSoftware -> CureOrTerminate.Cure (phase breach trigger)
```

Explainability for legal and operational decision-making.

### `repl` — interactive session

Load a contract, trigger events, query state, simulate, and trace cascades interactively.

```bash
kontra repl
```

```
kontra> load examples/software_license.k
Parties (2): licensee, licensor
Events (3): AcceptanceNotice, Delivery, Effective
...
Loaded examples/software_license.k

kontra> trigger Effective 2026-03-01
Triggered Effective on 2026-03-01

kontra> state_at 2026-04-15
DeliverSoftware: BREACHED (due 2026-04-10, OVERDUE by 5 days)
...

kontra> cascade DeliverSoftware
Cascade from 'DeliverSoftware':
- DeliverSoftware -> CureOrTerminate (remedy breach target)
- DeliverSoftware -> CureOrTerminate.Cure (phase breach trigger)
...
```

---

## Architecture

The pipeline follows the Crafting Interpreters Part II design — no AST, single-pass compilation to bytecode, then VM execution:

```
source.k → scanner → compiler → bytecode chunk → VM → ContractDef → runtime engine
```

| Module | Role |
|---|---|
| `scanner.rs` + `token.rs` | On-demand lexer with spans for error reporting |
| `compiler.rs` | Single-pass recursive descent — parses tokens and emits bytecode directly |
| `chunk.rs` | Bytecode container: opcodes, constant pool, source line mapping |
| `vm.rs` | Executes bytecode into a fully populated `ContractDef` |
| `engine.rs` | Runtime state machine — obligation lifecycle, event evaluation, deadline computation, simulation, breach cascade |
| `diff.rs` | Normalized semantic contract comparison and risk warning generation |
| `calendar.rs` + `config.rs` | Business-day arithmetic with configurable holiday calendars and jurisdiction support |
| `main.rs` | CLI surface: `run`, `eval`, `simulate`, `diff`, `cascade`, `repl` |

Obligations follow a state machine: `Pending → Active → Satisfied | Breached → Remedied`. The engine runs a fixed-point evaluation loop — it keeps transitioning states until nothing changes, which is how breach cascades propagate through remedy phases automatically.

Time is first-class. Business days and calendar days are distinct. Deadlines are jurisdiction-aware through configurable calendar registries with holiday support.

---

## Editor tooling

The companion **[kontra-lsp](https://github.com/mathew-bravo/kontra-lsp)** project provides Language Server Protocol support for `.k` files:

- **Diagnostics** — real-time compile errors as you type, clearing when fixed
- **Autocomplete** — DSL keywords (`contract`, `obligation`, `remedy`, `breach_of`, ...) and snippet templates for common patterns
- **Hover docs** — inline documentation for functions like `after()`, `satisfied()`, `business_days`
- **Neovim integration** — documented local setup with `nvim-lspconfig`

The LSP reuses kontra's compiler and scanner directly — same error messages, same validation, zero divergence.

---

## Business-day calendars

Deadlines can be jurisdiction-aware. Create a `kontra-calendars.json` to configure holidays and business weekdays:

```json
{
  "default_calendar_id": "us_ny",
  "calendars": [
    {
      "id": "us_ny",
      "jurisdiction": "US-NY",
      "business_weekdays": ["Mon", "Tue", "Wed", "Thu", "Fri"],
      "holidays": ["2026-03-09"]
    }
  ]
}
```

A term defined as `5 business_days from Effective` will skip weekends and configured holidays automatically.

---

## Built with

- **Rust** (2024 edition) — the entire system is a single crate with no parser generators
- **chrono** — date arithmetic
- **Hand-rolled scanner and compiler** — following Crafting Interpreters Chapters 16-25, adapted for a declarative contract DSL instead of an imperative language
- **109 tests** across scanner, compiler, VM, engine, diff, calendar, and CLI modules

```bash
cargo test
# test result: ok. 109 passed; 0 failed
```

---

## What matters

This isn't just parsing contract text. kontra executes obligations over time, simulates outcomes before decisions are made, detects risky negotiation edits, and explains downstream breach impact with deterministic output. That's the shift from static documents to operational legal infrastructure.
