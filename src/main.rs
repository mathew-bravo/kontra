use std::env;
use std::fs;
use std::io::{self, Write};

use chrono::NaiveDate;
use kontra::compiler::Compiler;
use kontra::config::load_calendar_registry;
use kontra::diff::{diff_contracts, generate_risk_warnings, render_contract_diff};
use kontra::engine::ContractRuntime;
use kontra::types::ContractDef;
use kontra::vm::VM;

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Err(err) = run_cli(args) {
        eprintln!("{}", err);
        std::process::exit(1);
    }
}

fn run_cli(args: Vec<String>) -> Result<(), String> {
    if args.is_empty() {
        print_help();
        return Ok(());
    }

    match args[0].as_str() {
        "run" => cmd_run(&args[1..]),
        "eval" => cmd_eval(&args[1..]),
        "simulate" => cmd_simulate(&args[1..]),
        "diff" => cmd_diff(&args[1..]),
        "cascade" => cmd_cascade(&args[1..]),
        "repl" => cmd_repl(&args[1..]),
        "help" | "-h" | "--help" => {
            print_help();
            Ok(())
        }
        other => Err(format!(
            "Unknown subcommand '{}'.\n\n{}",
            other,
            help_text()
        )),
    }
}

fn cmd_run(args: &[String]) -> Result<(), String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{}", run_help_text());
        return Ok(());
    }
    if args.len() != 1 {
        return Err(usage_error(
            "run expects exactly one contract file path.",
            run_help_text(),
        ));
    }

    let contract = load_contract_from_file(&args[0])?;
    print_contract_summary(&contract);
    Ok(())
}

fn cmd_eval(args: &[String]) -> Result<(), String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{}", eval_help_text());
        return Ok(());
    }
    if args.is_empty() {
        return Err(usage_error("eval requires a contract file path.", eval_help_text()));
    }

    let contract = load_contract_from_file(&args[0])?;
    let eval_options = parse_eval_options(&args[1..])?;
    let calendar_registry = load_calendar_registry()?;

    let mut runtime = ContractRuntime::with_calendar_registry(contract, calendar_registry)?;
    let mut latest_trigger_date: Option<NaiveDate> = None;

    for (event_name, date) in eval_options.triggers {
        runtime.trigger_event(&event_name, date);
        latest_trigger_date = Some(match latest_trigger_date {
            Some(prev) if prev > date => prev,
            _ => date,
        });
    }

    if let Some(date) = eval_options.at {
        runtime.evaluate_at(date);
    } else if let Some(date) = latest_trigger_date {
        runtime.evaluate_at(date);
    }

    for snapshot in runtime.query_state() {
        println!("{}", snapshot);
    }
    Ok(())
}

fn cmd_simulate(args: &[String]) -> Result<(), String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{}", simulate_help_text());
        return Ok(());
    }
    if args.is_empty() {
        return Err(usage_error(
            "simulate requires a contract file path.",
            simulate_help_text(),
        ));
    }

    let contract = load_contract_from_file(&args[0])?;
    let simulate_options = parse_simulate_options(&args[1..])?;
    let calendar_registry = load_calendar_registry()?;
    let runtime = ContractRuntime::with_calendar_registry(contract, calendar_registry)?;
    println!("{}", render_simulation_output(&runtime, &simulate_options));
    Ok(())
}

fn cmd_diff(args: &[String]) -> Result<(), String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{}", diff_help_text());
        return Ok(());
    }
    if args.len() != 2 {
        return Err(usage_error(
            "diff expects exactly two contract file paths: <old-file> <new-file>.",
            diff_help_text(),
        ));
    }

    let old_contract = load_contract_from_file(&args[0])?;
    let new_contract = load_contract_from_file(&args[1])?;

    let diff = diff_contracts(&old_contract, &new_contract);
    for line in render_contract_diff(&diff) {
        println!("{}", line);
    }

    let warnings = generate_risk_warnings(&old_contract, &new_contract, &diff);
    if !warnings.is_empty() {
        println!();
        for warning in warnings {
            println!("{}", warning);
        }
    }
    Ok(())
}

fn cmd_cascade(args: &[String]) -> Result<(), String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{}", cascade_help_text());
        return Ok(());
    }
    if args.len() != 2 {
        return Err(usage_error(
            "cascade expects a file path and runtime item name.",
            cascade_help_text(),
        ));
    }

    let contract = load_contract_from_file(&args[0])?;
    let calendar_registry = load_calendar_registry()?;
    let runtime = ContractRuntime::with_calendar_registry(contract, calendar_registry)?;
    let item_name = resolve_runtime_item_for_cascade(&runtime, &args[1])?;
    let cascade = runtime.trace_breach_cascade(&item_name);

    println!("Cascade from '{}':", cascade.root);
    if cascade.links.is_empty() {
        println!("(no downstream effects)");
    } else {
        for link in cascade.links {
            println!("- {} -> {} ({})", link.from, link.to, link.reason);
        }
    }
    Ok(())
}

fn cmd_repl(args: &[String]) -> Result<(), String> {
    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{}", repl_help_text());
        return Ok(());
    }
    if !args.is_empty() {
        return Err(usage_error(
            "repl does not take positional arguments.",
            repl_command_help_text(),
        ));
    }

    println!("kontra repl - type 'help' for commands");
    let mut runtime: Option<ContractRuntime> = None;

    loop {
        print!("kontra> ");
        io::stdout()
            .flush()
            .map_err(|e| format!("Failed to flush stdout: {}", e))?;

        let mut line = String::new();
        let bytes = io::stdin()
            .read_line(&mut line)
            .map_err(|e| format!("Failed to read from stdin: {}", e))?;
        if bytes == 0 {
            break; // EOF
        }

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts[0] {
            "help" => print_repl_help(),
            "quit" | "exit" => break,
            "load" => {
                if parts.len() != 2 {
                    eprintln!("Usage: load <file>");
                    continue;
                }
                match load_contract_from_file(parts[1]) {
                    Ok(contract) => {
                        match load_calendar_registry().and_then(|registry| {
                            ContractRuntime::with_calendar_registry(contract, registry)
                        }) {
                            Ok(loaded_runtime) => {
                                print_contract_summary(&loaded_runtime.contract);
                                runtime = Some(loaded_runtime);
                                println!("Loaded {}", parts[1]);
                            }
                            Err(err) => eprintln!("{}", err),
                        }
                    }
                    Err(err) => eprintln!("{}", err),
                }
            }
            "trigger" => {
                if parts.len() != 3 {
                    eprintln!("Usage: trigger <event> <YYYY-MM-DD>");
                    continue;
                }
                let Some(rt) = runtime.as_mut() else {
                    eprintln!("No contract loaded. Use: load <file>");
                    continue;
                };
                match parse_date(parts[2]) {
                    Ok(date) => {
                        rt.trigger_event(parts[1], date);
                        println!("Triggered {} on {}", parts[1], date);
                    }
                    Err(err) => eprintln!("{}", err),
                }
            }
            "state_at" => {
                if parts.len() != 2 {
                    eprintln!("Usage: state_at <YYYY-MM-DD>");
                    continue;
                }
                let Some(rt) = runtime.as_mut() else {
                    eprintln!("No contract loaded. Use: load <file>");
                    continue;
                };
                match parse_date(parts[1]) {
                    Ok(date) => {
                        rt.evaluate_at(date);
                        for snapshot in rt.query_state() {
                            println!("{}", snapshot);
                        }
                    }
                    Err(err) => eprintln!("{}", err),
                }
            }
            "satisfy" => {
                if parts.len() != 2 {
                    eprintln!("Usage: satisfy <obligation>");
                    continue;
                }
                let Some(rt) = runtime.as_mut() else {
                    eprintln!("No contract loaded. Use: load <file>");
                    continue;
                };
                rt.satisfy(parts[1]);
                println!("Marked {} as satisfied (if it exists)", parts[1]);
            }
            "simulate" => {
                let Some(rt) = runtime.as_ref() else {
                    eprintln!("No contract loaded. Use: load <file>");
                    continue;
                };
                let simulate_args: Vec<String> =
                    parts[1..].iter().map(|segment| (*segment).to_string()).collect();
                match parse_simulate_options(&simulate_args) {
                    Ok(options) => {
                        println!("{}", render_simulation_output(rt, &options));
                    }
                    Err(err) => eprintln!("{}", err),
                }
            }
            "cascade" => {
                if parts.len() != 2 {
                    eprintln!("Usage: cascade <runtime-item>");
                    continue;
                }
                let Some(rt) = runtime.as_ref() else {
                    eprintln!("No contract loaded. Use: load <file>");
                    continue;
                };
                match resolve_runtime_item_for_cascade(rt, parts[1]) {
                    Ok(item_name) => {
                        let cascade = rt.trace_breach_cascade(&item_name);
                        println!("Cascade from '{}':", cascade.root);
                        if cascade.links.is_empty() {
                            println!("(no downstream effects)");
                        } else {
                            for link in cascade.links {
                                println!("- {} -> {} ({})", link.from, link.to, link.reason);
                            }
                        }
                    }
                    Err(err) => eprintln!("{}", err),
                }
            }
            _ => {
                eprintln!("Unknown command. Type 'help' for commands.");
            }
        }
    }

    Ok(())
}

fn parse_eval_options(args: &[String]) -> Result<EvalOptions, String> {
    parse_runtime_options(args, "eval", eval_help_text(), false)
}

fn parse_simulate_options(args: &[String]) -> Result<EvalOptions, String> {
    parse_runtime_options(args, "simulate", simulate_help_text(), true)
}

fn parse_runtime_options(
    args: &[String],
    command_name: &str,
    help_text: &str,
    require_trigger: bool,
) -> Result<EvalOptions, String> {
    let mut triggers = Vec::new();
    let mut at: Option<NaiveDate> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--trigger" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| usage_error("Expected value after --trigger.", help_text))?;
                triggers.push(parse_trigger_arg(raw)?);
                i += 2;
            }
            "--at" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| usage_error("Expected value after --at.", help_text))?;
                at = Some(parse_date(raw)?);
                i += 2;
            }
            unknown => {
                return Err(usage_error(
                    &format!("Unknown {} argument '{}'.", command_name, unknown),
                    help_text,
                ));
            }
        }
    }

    if require_trigger && triggers.is_empty() {
        return Err(usage_error(
            &format!("At least one --trigger is required for {}.", command_name),
            help_text,
        ));
    }

    Ok(EvalOptions { triggers, at })
}

fn parse_date(raw: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| format!("Invalid date '{}'. Expected YYYY-MM-DD.", raw))
}

fn parse_trigger_arg(raw: &str) -> Result<(String, NaiveDate), String> {
    let (event_name, date_raw) = raw.split_once('=').ok_or_else(|| {
        format!(
            "Invalid trigger '{}'. Expected EventName=YYYY-MM-DD.",
            raw
        )
    })?;

    if event_name.trim().is_empty() {
        return Err("Trigger event name cannot be empty".to_string());
    }

    let date = parse_date(date_raw.trim())?;
    Ok((event_name.trim().to_string(), date))
}

fn resolve_runtime_item_for_cascade(runtime: &ContractRuntime, raw: &str) -> Result<String, String> {
    if runtime.states.contains_key(raw) {
        return Ok(raw.to_string());
    }

    let suffix = format!(".{}", raw);
    let mut matches: Vec<String> = runtime
        .states
        .keys()
        .filter(|key| key.ends_with(&suffix))
        .cloned()
        .collect();
    matches.sort();

    match matches.as_slice() {
        [single] => Ok(single.clone()),
        [] => Err(format!(
            "Unknown runtime item '{}'. Use `kontra run <file>` to list obligations/remedies/phases.",
            raw
        )),
        _ => Err(format!(
            "Ambiguous runtime item '{}'. Matches: {}",
            raw,
            matches.join(", ")
        )),
    }
}

fn load_contract_from_file(path: &str) -> Result<ContractDef, String> {
    let source = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read contract file '{}': {}", path, e))?;
    let chunk = Compiler::compile(&source)
        .map_err(|e| format!("Compile failure while loading '{}': {}", path, e))?;
    VM::interpret(chunk).map_err(|e| format!("Runtime failure while loading '{}': {}", path, e))
}

fn print_contract_summary(contract: &ContractDef) {
    let mut parties: Vec<String> = contract.parties.keys().cloned().collect();
    let mut events: Vec<String> = contract.events.keys().cloned().collect();
    let mut terms: Vec<String> = contract.terms.keys().cloned().collect();
    let mut obligations: Vec<String> = contract.obligations.keys().cloned().collect();
    let mut remedies: Vec<String> = contract.remedies.keys().cloned().collect();

    let mut phases: Vec<String> = contract
        .remedies
        .values()
        .flat_map(|remedy| {
            remedy
                .phases
                .iter()
                .map(move |phase| format!("{}.{}", remedy.name, phase.name))
        })
        .collect();

    parties.sort();
    events.sort();
    terms.sort();
    obligations.sort();
    remedies.sort();
    phases.sort();

    println!("Parties ({}): {}", parties.len(), join_list(&parties));
    println!("Events ({}): {}", events.len(), join_list(&events));
    println!("Terms ({}): {}", terms.len(), join_list(&terms));
    println!("Obligations ({}): {}", obligations.len(), join_list(&obligations));
    println!("Remedies ({}): {}", remedies.len(), join_list(&remedies));
    println!("Phases ({}): {}", phases.len(), join_list(&phases));
}

fn join_list(items: &[String]) -> String {
    if items.is_empty() {
        "(none)".to_string()
    } else {
        items.join(", ")
    }
}

fn print_help() {
    println!("{}", help_text());
}

fn help_text() -> &'static str {
    "kontra - contracts as code

Usage:
  kontra run <file>
  kontra eval <file> [--trigger Event=YYYY-MM-DD ...] [--at YYYY-MM-DD]
  kontra simulate <file> --trigger Event=YYYY-MM-DD [--trigger Event=YYYY-MM-DD ...] [--at YYYY-MM-DD]
  kontra diff <old-file> <new-file>
  kontra cascade <file> <runtime-item>
  kontra repl

Examples:
  kontra run examples/software_license.k
  kontra eval examples/software_license.k --trigger Effective=2026-03-01 --at 2026-04-15
  kontra simulate examples/software_license.k --trigger Delivery=2026-03-20 --at 2026-04-15
  kontra diff examples/software_license_v1.k examples/software_license_v2.k
  kontra cascade examples/software_license.k DeliverSoftware
  kontra repl
"
}

fn run_help_text() -> &'static str {
    "Usage:
  kontra run <file>

Description:
  Compile and execute a .k file, then print a structural contract summary.
"
}

fn eval_help_text() -> &'static str {
    "Usage:
  kontra eval <file> [--trigger Event=YYYY-MM-DD ...] [--at YYYY-MM-DD]

Flags:
  --trigger Event=YYYY-MM-DD   Trigger an event; repeatable
  --at YYYY-MM-DD              Evaluate state at a specific date
"
}

fn simulate_help_text() -> &'static str {
    "Usage:
  kontra simulate <file> --trigger Event=YYYY-MM-DD [--trigger Event=YYYY-MM-DD ...] [--at YYYY-MM-DD]

Flags:
  --trigger Event=YYYY-MM-DD   Hypothetical event; repeatable (required)
  --at YYYY-MM-DD              Evaluate simulated state at a specific date
"
}

fn diff_help_text() -> &'static str {
    "Usage:
  kontra diff <old-file> <new-file>

Output:
  ADDED / REMOVED / CHANGED entries for parties, events, terms, obligations, remedies, and phases
  plus WARNING lines for high-risk changes
"
}

fn cascade_help_text() -> &'static str {
    "Usage:
  kontra cascade <file> <runtime-item>

Examples:
  kontra cascade examples/software_license.k DeliverSoftware
  kontra cascade examples/software_license.k CureOrTerminate.Cure
"
}

fn repl_command_help_text() -> &'static str {
    "Usage:
  kontra repl

Description:
  Start an interactive session for load/trigger/state_at/simulate/cascade/satisfy commands.
"
}

fn usage_error(message: &str, help_text: &str) -> String {
    format!("Usage error: {}\n\n{}", message, help_text)
}

fn render_simulation_output(runtime: &ContractRuntime, options: &EvalOptions) -> String {
    let baseline_events: Vec<(String, NaiveDate)> = Vec::new();
    let canonical = runtime.simulate_with(&baseline_events, options.at);
    let simulated = runtime.simulate_with(&options.triggers, options.at);

    let canonical_lines: Vec<String> = canonical
        .query_state()
        .into_iter()
        .map(|snapshot| snapshot.to_string())
        .collect();
    let simulated_lines: Vec<String> = simulated
        .query_state()
        .into_iter()
        .map(|snapshot| snapshot.to_string())
        .collect();

    format_simulation_output(&canonical_lines, &simulated_lines)
}

fn format_simulation_output(canonical_lines: &[String], simulated_lines: &[String]) -> String {
    let canonical_body = if canonical_lines.is_empty() {
        "(none)".to_string()
    } else {
        canonical_lines.join("\n")
    };
    let simulated_body = if simulated_lines.is_empty() {
        "(none)".to_string()
    } else {
        simulated_lines.join("\n")
    };

    format!(
        "Canonical state:\n{}\n\nSimulated state:\n{}",
        canonical_body, simulated_body
    )
}

fn print_repl_help() {
    println!("{}", repl_help_text());
}

fn repl_help_text() -> &'static str {
    "Commands:
  load <file>                    Load and summarize a contract
  trigger <event> <YYYY-MM-DD>   Record an event occurrence
  state_at <YYYY-MM-DD>          Evaluate and print runtime states
  simulate --trigger Event=YYYY-MM-DD [--trigger Event=YYYY-MM-DD ...] [--at YYYY-MM-DD]
                                 Run non-destructive hypothetical evaluation
  cascade <runtime-item>         Show downstream breach impacts
  satisfy <obligation>           Mark an obligation/phase/remedy item as satisfied
  help                           Show this help
  quit                           Exit the REPL

Examples:
  load examples/software_license.k
  trigger Effective 2026-03-01
  state_at 2026-04-15
  simulate --trigger Delivery=2026-03-20 --at 2026-04-15
  cascade DeliverSoftware
  satisfy DeliverSoftware"
}

#[derive(Debug)]
struct EvalOptions {
    triggers: Vec<(String, NaiveDate)>,
    at: Option<NaiveDate>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use kontra::compiler::Compiler;
    use kontra::types::CalendarRegistry;
    use kontra::vm::VM;

    fn runtime_from_source(source: &str) -> ContractRuntime {
        let chunk = Compiler::compile(source).expect("compile should succeed");
        let contract = VM::interpret(chunk).expect("vm interpret should succeed");
        let registry = CalendarRegistry::phase2_default();
        ContractRuntime::with_calendar_registry(contract, registry)
            .expect("runtime should build from source")
    }

    #[test]
    fn parse_date_accepts_valid_iso() {
        let date = parse_date("2026-03-01").expect("should parse");
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid"));
    }

    #[test]
    fn parse_date_rejects_invalid_format() {
        let err = parse_date("03/01/2026").expect_err("should reject non-iso date");
        assert!(err.contains("Expected YYYY-MM-DD"));
    }

    #[test]
    fn parse_trigger_arg_accepts_valid_pair() {
        let (name, date) =
            parse_trigger_arg("Effective=2026-03-01").expect("should parse trigger");
        assert_eq!(name, "Effective");
        assert_eq!(date, NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid"));
    }

    #[test]
    fn parse_trigger_arg_rejects_missing_equal() {
        let err = parse_trigger_arg("Effective-2026-03-01").expect_err("should fail");
        assert!(err.contains("Expected EventName=YYYY-MM-DD"));
    }

    #[test]
    fn parse_eval_options_parses_repeated_trigger_and_at() {
        let args = vec![
            "--trigger".to_string(),
            "Effective=2026-03-01".to_string(),
            "--trigger".to_string(),
            "Delivery=2026-03-15".to_string(),
            "--at".to_string(),
            "2026-04-01".to_string(),
        ];

        let options = parse_eval_options(&args).expect("options should parse");
        assert_eq!(options.triggers.len(), 2);
        assert_eq!(options.triggers[0].0, "Effective");
        assert_eq!(options.triggers[1].0, "Delivery");
        assert_eq!(
            options.at,
            Some(NaiveDate::from_ymd_opt(2026, 4, 1).expect("valid date"))
        );
    }

    #[test]
    fn parse_eval_options_rejects_unknown_flag() {
        let args = vec!["--oops".to_string()];
        let err = parse_eval_options(&args).expect_err("unknown flag should fail");
        assert!(err.contains("Unknown eval argument"));
    }

    #[test]
    fn parse_simulate_options_parses_repeated_trigger_and_at() {
        let args = vec![
            "--trigger".to_string(),
            "Effective=2026-03-01".to_string(),
            "--trigger".to_string(),
            "Delivery=2026-03-15".to_string(),
            "--at".to_string(),
            "2026-04-01".to_string(),
        ];

        let options = parse_simulate_options(&args).expect("simulate options should parse");
        assert_eq!(options.triggers.len(), 2);
        assert_eq!(options.triggers[0].0, "Effective");
        assert_eq!(options.triggers[1].0, "Delivery");
        assert_eq!(
            options.at,
            Some(NaiveDate::from_ymd_opt(2026, 4, 1).expect("valid date"))
        );
    }

    #[test]
    fn parse_simulate_options_requires_trigger() {
        let args = vec!["--at".to_string(), "2026-04-01".to_string()];
        let err = parse_simulate_options(&args).expect_err("simulate should require trigger");
        assert!(err.contains("At least one --trigger is required for simulate"));
    }

    #[test]
    fn parse_simulate_options_rejects_unknown_flag() {
        let args = vec!["--oops".to_string()];
        let err = parse_simulate_options(&args).expect_err("unknown flag should fail");
        assert!(err.contains("Usage error"));
        assert!(err.contains("Unknown simulate argument"));
    }

    #[test]
    fn format_simulation_output_labels_canonical_and_simulated_sections() {
        let canonical = vec!["Deliver: PENDING (blocked - condition not met)".to_string()];
        let simulated = vec!["Deliver: ACTIVE (due 2026-03-15)".to_string()];

        let output = format_simulation_output(&canonical, &simulated);
        assert!(output.contains("Canonical state:"));
        assert!(output.contains("Simulated state:"));
        assert!(output.contains("Deliver: PENDING"));
        assert!(output.contains("Deliver: ACTIVE"));
    }

    #[test]
    fn render_simulation_output_matches_expected_golden_flow() {
        let runtime = runtime_from_source(
            r#"contract Foo {
                term DeliveryWindow = 1 calendar_days from Effective
                obligation Deliver {
                    party: seller
                    action: "Deliver software"
                    due: DeliveryWindow
                    condition: after(Effective)
                }
            }"#,
        );
        let options = parse_simulate_options(&[
            "--trigger".to_string(),
            "Effective=2026-03-01".to_string(),
            "--at".to_string(),
            "2026-03-01".to_string(),
        ])
        .expect("simulate options should parse");

        let output = render_simulation_output(&runtime, &options);
        let expected = "Canonical state:
Deliver: PENDING (blocked - condition not met)

Simulated state:
Deliver: ACTIVE (due 2026-03-02)";
        assert_eq!(output, expected);
    }

    #[test]
    fn parse_simulate_options_parses_multiple_triggers_without_at() {
        let args = vec![
            "--trigger".to_string(),
            "Effective=2026-03-01".to_string(),
            "--trigger".to_string(),
            "Delivery=2026-03-20".to_string(),
        ];

        let options = parse_simulate_options(&args).expect("simulate options should parse");
        assert_eq!(options.triggers.len(), 2);
        assert_eq!(options.at, None);
    }

    #[test]
    fn parse_eval_options_rejects_malformed_at_date() {
        let args = vec!["--at".to_string(), "03-01-2026".to_string()];
        let err = parse_eval_options(&args).expect_err("malformed date should fail");
        assert!(err.contains("Invalid date"));
        assert!(err.contains("YYYY-MM-DD"));
    }

    #[test]
    fn run_cli_unknown_subcommand_is_actionable() {
        let err = run_cli(vec!["unknown".to_string()]).expect_err("unknown subcommand should fail");
        assert!(err.contains("Unknown subcommand 'unknown'"));
        assert!(err.contains("Usage:"));
    }

    #[test]
    fn run_help_text_has_clear_usage() {
        let text = run_help_text();
        assert!(text.contains("Usage:"));
        assert!(text.contains("kontra run <file>"));
    }

    #[test]
    fn repl_help_text_includes_simulate_cascade_and_examples() {
        let text = repl_help_text();
        assert!(text.contains("simulate --trigger"));
        assert!(text.contains("cascade <runtime-item>"));
        assert!(text.contains("Examples:"));
        assert!(text.contains("load examples/software_license.k"));
    }

    #[test]
    fn resolve_runtime_item_for_cascade_rejects_unknown_item() {
        let runtime = runtime_from_source(
            r#"contract Foo {
                obligation DeliverSoftware { action: "Deliver software" }
            }"#,
        );

        let err = resolve_runtime_item_for_cascade(&runtime, "MissingItem")
            .expect_err("unknown item should fail");
        assert!(err.contains("Unknown runtime item 'MissingItem'"));
    }

    #[test]
    fn resolve_runtime_item_for_cascade_supports_unique_suffix_match() {
        let runtime = runtime_from_source(
            r#"contract Foo {
                obligation DeliverSoftware { action: "Deliver software" }
                remedy CureOrTerminate on breach_of(DeliverSoftware) {
                    phase Cure { action: "Cure" }
                }
            }"#,
        );

        let resolved =
            resolve_runtime_item_for_cascade(&runtime, "Cure").expect("single suffix should resolve");
        assert_eq!(resolved, "CureOrTerminate.Cure");
    }

    #[test]
    fn resolve_runtime_item_for_cascade_rejects_ambiguous_suffix_match() {
        let runtime = runtime_from_source(
            r#"contract Foo {
                obligation DeliverSoftware { action: "Deliver software" }
                obligation PayFee { action: "Pay fee" }
                remedy CureA on breach_of(DeliverSoftware) { phase Cure { action: "Cure A" } }
                remedy CureB on breach_of(PayFee) { phase Cure { action: "Cure B" } }
            }"#,
        );

        let err = resolve_runtime_item_for_cascade(&runtime, "Cure")
            .expect_err("ambiguous suffix should fail");
        assert!(err.contains("Ambiguous runtime item 'Cure'"));
        assert!(err.contains("CureA.Cure"));
        assert!(err.contains("CureB.Cure"));
    }
}
