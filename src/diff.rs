use std::collections::BTreeSet;

use crate::types::{ContractDef, EventDef, ObligationDef, Party, PhaseDef, RemedyDef, Term};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldChange {
    pub field: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemDiffKind {
    Added,
    Removed,
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemDiff {
    pub key: String,
    pub kind: ItemDiffKind,
    pub changes: Vec<FieldChange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractDiff {
    pub parties: Vec<ItemDiff>,
    pub events: Vec<ItemDiff>,
    pub terms: Vec<ItemDiff>,
    pub obligations: Vec<ItemDiff>,
    pub remedies: Vec<ItemDiff>,
    pub phases: Vec<ItemDiff>,
}

pub fn diff_contracts(old_contract: &ContractDef, new_contract: &ContractDef) -> ContractDiff {
    let old_phases = flatten_phases(old_contract);
    let new_phases = flatten_phases(new_contract);

    ContractDiff {
        parties: diff_named_maps(
            old_contract.parties.iter().map(|(k, v)| (k.as_str(), v)),
            new_contract.parties.iter().map(|(k, v)| (k.as_str(), v)),
            compare_party,
        ),
        events: diff_named_maps(
            old_contract.events.iter().map(|(k, v)| (k.as_str(), v)),
            new_contract.events.iter().map(|(k, v)| (k.as_str(), v)),
            compare_event,
        ),
        terms: diff_named_maps(
            old_contract.terms.iter().map(|(k, v)| (k.as_str(), v)),
            new_contract.terms.iter().map(|(k, v)| (k.as_str(), v)),
            compare_term,
        ),
        obligations: diff_named_maps(
            old_contract.obligations.iter().map(|(k, v)| (k.as_str(), v)),
            new_contract.obligations.iter().map(|(k, v)| (k.as_str(), v)),
            compare_obligation,
        ),
        remedies: diff_named_maps(
            old_contract.remedies.iter().map(|(k, v)| (k.as_str(), v)),
            new_contract.remedies.iter().map(|(k, v)| (k.as_str(), v)),
            compare_remedy,
        ),
        phases: diff_named_maps(
            old_phases.iter().map(|(key, phase)| (key.as_str(), *phase)),
            new_phases.iter().map(|(key, phase)| (key.as_str(), *phase)),
            compare_phase,
        ),
    }
}

pub fn render_contract_diff(diff: &ContractDiff) -> Vec<String> {
    let mut lines = Vec::new();
    append_category_lines("PARTY", &diff.parties, &mut lines);
    append_category_lines("EVENT", &diff.events, &mut lines);
    append_category_lines("TERM", &diff.terms, &mut lines);
    append_category_lines("OBLIGATION", &diff.obligations, &mut lines);
    append_category_lines("REMEDY", &diff.remedies, &mut lines);
    append_category_lines("PHASE", &diff.phases, &mut lines);

    if lines.is_empty() {
        vec!["No changes detected.".to_string()]
    } else {
        lines
    }
}

pub fn generate_risk_warnings(
    old_contract: &ContractDef,
    new_contract: &ContractDef,
    diff: &ContractDiff,
) -> Vec<String> {
    let mut warnings: BTreeSet<String> = BTreeSet::new();

    for obligation_name in old_contract.obligations.keys() {
        let old_has = old_contract
            .remedies
            .values()
            .any(|remedy| remedy.breach_target == *obligation_name);
        let new_has = new_contract
            .remedies
            .values()
            .any(|remedy| remedy.breach_target == *obligation_name);

        if old_has && !new_has {
            warnings.insert(format!(
                "WARNING: breach of '{}' now has no remedy",
                obligation_name
            ));
        }
    }

    for phase_diff in &diff.phases {
        if phase_diff.kind == ItemDiffKind::Removed {
            warnings.insert(format!("WARNING: removed remedy phase '{}'", phase_diff.key));
        }
    }

    warnings.into_iter().collect()
}

fn diff_named_maps<'a, T: PartialEq + 'a>(
    old_items: impl Iterator<Item = (&'a str, &'a T)>,
    new_items: impl Iterator<Item = (&'a str, &'a T)>,
    compare_fields: fn(&T, &T) -> Vec<FieldChange>,
) -> Vec<ItemDiff> {
    let old_vec: Vec<(&str, &T)> = old_items.collect();
    let new_vec: Vec<(&str, &T)> = new_items.collect();

    let old_keys: BTreeSet<String> = old_vec.iter().map(|(k, _)| (*k).to_string()).collect();
    let new_keys: BTreeSet<String> = new_vec.iter().map(|(k, _)| (*k).to_string()).collect();
    let all_keys: BTreeSet<String> = old_keys.union(&new_keys).cloned().collect();

    let mut diffs = Vec::new();
    for key in all_keys {
        let old_item = old_vec.iter().find(|(k, _)| *k == key.as_str()).map(|(_, v)| *v);
        let new_item = new_vec.iter().find(|(k, _)| *k == key.as_str()).map(|(_, v)| *v);

        match (old_item, new_item) {
            (None, Some(_)) => diffs.push(ItemDiff {
                key,
                kind: ItemDiffKind::Added,
                changes: Vec::new(),
            }),
            (Some(_), None) => diffs.push(ItemDiff {
                key,
                kind: ItemDiffKind::Removed,
                changes: Vec::new(),
            }),
            (Some(old), Some(new)) if old != new => diffs.push(ItemDiff {
                key,
                kind: ItemDiffKind::Changed,
                changes: compare_fields(old, new),
            }),
            _ => {}
        }
    }

    diffs
}

fn append_category_lines(category: &str, diffs: &[ItemDiff], lines: &mut Vec<String>) {
    for item in diffs {
        match item.kind {
            ItemDiffKind::Added => lines.push(format!("ADDED: {} {}", category, item.key)),
            ItemDiffKind::Removed => lines.push(format!("REMOVED: {} {}", category, item.key)),
            ItemDiffKind::Changed => {
                if item.changes.is_empty() {
                    lines.push(format!("CHANGED: {} {}", category, item.key));
                    continue;
                }
                for change in &item.changes {
                    lines.push(format!(
                        "CHANGED: {} {} — {}: {} -> {}",
                        category, item.key, change.field, change.old, change.new
                    ));
                }
            }
        }
    }
}

fn flatten_phases(contract: &ContractDef) -> Vec<(String, &PhaseDef)> {
    contract.remedies.values().flat_map(|remedy| {
        remedy
            .phases
            .iter()
            .map(move |phase| (format!("{}.{}", remedy.name, phase.name), phase))
    })
    .collect()
}

fn compare_party(old: &Party, new: &Party) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    push_if_changed(&mut changes, "role", &old.role, &new.role);
    push_if_changed(&mut changes, "name", &old.name, &new.name);
    changes
}

fn compare_event(old: &EventDef, new: &EventDef) -> Vec<FieldChange> {
    match (old, new) {
        (EventDef::DateEvent(old_date), EventDef::DateEvent(new_date)) => {
            diff_single_field("date", old_date, new_date)
        }
        (EventDef::TriggeredEvent(old_role), EventDef::TriggeredEvent(new_role)) => {
            diff_single_field("triggered_by", old_role, new_role)
        }
        _ => vec![
            FieldChange {
                field: "kind".to_string(),
                old: format!("{:?}", old),
                new: format!("{:?}", new),
            },
            FieldChange {
                field: "value".to_string(),
                old: format!("{:?}", old),
                new: format!("{:?}", new),
            },
        ],
    }
}

fn compare_term(old: &Term, new: &Term) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    push_if_changed(&mut changes, "name", &old.name, &new.name);
    push_if_changed(
        &mut changes,
        "duration.amount",
        &old.duration.amount,
        &new.duration.amount,
    );
    push_if_changed(
        &mut changes,
        "duration.unit",
        &format!("{:?}", old.duration.unit),
        &format!("{:?}", new.duration.unit),
    );
    push_if_changed(
        &mut changes,
        "anchor",
        &format!("{:?}", old.anchor),
        &format!("{:?}", new.anchor),
    );
    push_if_changed(
        &mut changes,
        "calendar_ref",
        &format!("{:?}", old.calendar_ref),
        &format!("{:?}", new.calendar_ref),
    );
    changes
}

fn compare_obligation(old: &ObligationDef, new: &ObligationDef) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    push_if_changed(&mut changes, "name", &old.name, &new.name);
    push_if_changed(
        &mut changes,
        "party_role",
        &format!("{:?}", old.party_role),
        &format!("{:?}", new.party_role),
    );
    push_if_changed(
        &mut changes,
        "action",
        &format!("{:?}", old.action),
        &format!("{:?}", new.action),
    );
    push_if_changed(
        &mut changes,
        "due",
        &format!("{:?}", old.due),
        &format!("{:?}", new.due),
    );
    push_if_changed(
        &mut changes,
        "condition",
        &format!("{:?}", old.condition),
        &format!("{:?}", new.condition),
    );
    changes
}

fn compare_remedy(old: &RemedyDef, new: &RemedyDef) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    push_if_changed(&mut changes, "name", &old.name, &new.name);
    push_if_changed(
        &mut changes,
        "breach_target",
        &old.breach_target,
        &new.breach_target,
    );
    push_if_changed(
        &mut changes,
        "party_role",
        &format!("{:?}", old.party_role),
        &format!("{:?}", new.party_role),
    );
    push_if_changed(
        &mut changes,
        "action",
        &format!("{:?}", old.action),
        &format!("{:?}", new.action),
    );
    push_if_changed(
        &mut changes,
        "due",
        &format!("{:?}", old.due),
        &format!("{:?}", new.due),
    );
    push_if_changed(
        &mut changes,
        "condition",
        &format!("{:?}", old.condition),
        &format!("{:?}", new.condition),
    );
    push_if_changed(
        &mut changes,
        "phase_count",
        &old.phases.len(),
        &new.phases.len(),
    );
    changes
}

fn compare_phase(old: &PhaseDef, new: &PhaseDef) -> Vec<FieldChange> {
    let mut changes = Vec::new();
    push_if_changed(&mut changes, "name", &old.name, &new.name);
    push_if_changed(
        &mut changes,
        "breach_target",
        &format!("{:?}", old.breach_target),
        &format!("{:?}", new.breach_target),
    );
    push_if_changed(
        &mut changes,
        "party_role",
        &format!("{:?}", old.party_role),
        &format!("{:?}", new.party_role),
    );
    push_if_changed(
        &mut changes,
        "action",
        &format!("{:?}", old.action),
        &format!("{:?}", new.action),
    );
    push_if_changed(
        &mut changes,
        "due",
        &format!("{:?}", old.due),
        &format!("{:?}", new.due),
    );
    push_if_changed(
        &mut changes,
        "condition",
        &format!("{:?}", old.condition),
        &format!("{:?}", new.condition),
    );
    changes
}

fn diff_single_field<T: ToString + PartialEq>(field: &str, old: &T, new: &T) -> Vec<FieldChange> {
    if old == new {
        Vec::new()
    } else {
        vec![FieldChange {
            field: field.to_string(),
            old: old.to_string(),
            new: new.to_string(),
        }]
    }
}

fn push_if_changed<T: ToString + PartialEq>(
    changes: &mut Vec<FieldChange>,
    field: &str,
    old: &T,
    new: &T,
) {
    if old != new {
        changes.push(FieldChange {
            field: field.to_string(),
            old: old.to_string(),
            new: new.to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::compiler::Compiler;
    use crate::vm::VM;

    use super::*;

    fn contract_from_source(source: &str) -> ContractDef {
        let chunk = Compiler::compile(source).expect("compile should succeed");
        VM::interpret(chunk).expect("vm interpret should succeed")
    }

    #[test]
    fn diff_parties_detects_field_level_change() {
        let old = contract_from_source(r#"contract C { parties { buyer: "Alice" } }"#);
        let new = contract_from_source(r#"contract C { parties { buyer: "Alicia" } }"#);

        let diff = diff_contracts(&old, &new);
        assert_eq!(diff.parties.len(), 1);
        assert_eq!(diff.parties[0].kind, ItemDiffKind::Changed);
        assert_eq!(diff.parties[0].key, "buyer");
        assert_eq!(diff.parties[0].changes[0].field, "name");
    }

    #[test]
    fn diff_events_detects_changed_definition() {
        let old = contract_from_source(r#"contract C { event Effective = date("2026-03-01") }"#);
        let new = contract_from_source(r#"contract C { event Effective = date("2026-03-15") }"#);

        let diff = diff_contracts(&old, &new);
        assert_eq!(diff.events.len(), 1);
        assert_eq!(diff.events[0].kind, ItemDiffKind::Changed);
        assert_eq!(diff.events[0].changes[0].field, "date");
    }

    #[test]
    fn diff_terms_detects_duration_change() {
        let old = contract_from_source(
            r#"contract C { term DeliveryWindow = 10 calendar_days from Effective }"#,
        );
        let new = contract_from_source(
            r#"contract C { term DeliveryWindow = 15 calendar_days from Effective }"#,
        );

        let diff = diff_contracts(&old, &new);
        assert_eq!(diff.terms.len(), 1);
        assert_eq!(diff.terms[0].kind, ItemDiffKind::Changed);
        assert!(
            diff.terms[0]
                .changes
                .iter()
                .any(|change| change.field == "duration.amount")
        );
    }

    #[test]
    fn diff_obligations_detects_condition_change() {
        let old = contract_from_source(
            r#"contract C {
                obligation Pay {
                    party: buyer
                    action: "Pay fee"
                    condition: occurred(AcceptanceNotice)
                }
            }"#,
        );
        let new = contract_from_source(
            r#"contract C {
                obligation Pay {
                    party: buyer
                    action: "Pay fee"
                    condition: satisfied(DeliverSoftware)
                }
            }"#,
        );

        let diff = diff_contracts(&old, &new);
        assert_eq!(diff.obligations.len(), 1);
        assert_eq!(diff.obligations[0].kind, ItemDiffKind::Changed);
        assert!(
            diff.obligations[0]
                .changes
                .iter()
                .any(|change| change.field == "condition")
        );
    }

    #[test]
    fn diff_remedies_detects_breach_target_change() {
        let old = contract_from_source(
            r#"contract C {
                remedy LateFee on breach_of(PayFee) { action: "Charge fee" }
            }"#,
        );
        let new = contract_from_source(
            r#"contract C {
                remedy LateFee on breach_of(PayInvoice) { action: "Charge fee" }
            }"#,
        );

        let diff = diff_contracts(&old, &new);
        assert_eq!(diff.remedies.len(), 1);
        assert_eq!(diff.remedies[0].kind, ItemDiffKind::Changed);
        assert!(
            diff.remedies[0]
                .changes
                .iter()
                .any(|change| change.field == "breach_target")
        );
    }

    #[test]
    fn diff_phases_detects_phase_field_change() {
        let old = contract_from_source(
            r#"contract C {
                remedy CureOrTerminate on breach_of(DeliverSoftware) {
                    phase Cure { action: "Deliver software" }
                }
            }"#,
        );
        let new = contract_from_source(
            r#"contract C {
                remedy CureOrTerminate on breach_of(DeliverSoftware) {
                    phase Cure { action: "Deliver software in cure period" }
                }
            }"#,
        );

        let diff = diff_contracts(&old, &new);
        assert_eq!(diff.phases.len(), 1);
        assert_eq!(diff.phases[0].kind, ItemDiffKind::Changed);
        assert!(
            diff.phases[0]
                .changes
                .iter()
                .any(|change| change.field == "action")
        );
    }

    #[test]
    fn diff_ordering_is_deterministic_independent_of_declaration_order() {
        let old = contract_from_source(
            r#"contract C {
                event B = date("2026-03-01")
                event A = date("2026-03-01")
            }"#,
        );
        let new = contract_from_source(
            r#"contract C {
                event A = date("2026-03-02")
                event B = date("2026-03-03")
            }"#,
        );

        let diff = diff_contracts(&old, &new);
        assert_eq!(diff.events.len(), 2);
        assert_eq!(diff.events[0].key, "A");
        assert_eq!(diff.events[1].key, "B");
    }

    #[test]
    fn render_diff_includes_changed_term_added_obligation_and_removed_phase() {
        let old = contract_from_source(
            r#"contract C {
                term DeliveryPeriod = 30 business_days from Effective
                obligation DeliverSoftware { action: "Deliver software" }
                remedy CureOrTerminate on breach_of(DeliverSoftware) {
                    phase Cure { action: "Cure" }
                }
            }"#,
        );
        let new = contract_from_source(
            r#"contract C {
                term DeliveryPeriod = 45 business_days from Effective
                obligation DeliverSoftware { action: "Deliver software" }
                obligation AuditRights { action: "Allow audit annually" }
                remedy CureOrTerminate on breach_of(DeliverSoftware) {}
            }"#,
        );

        let diff = diff_contracts(&old, &new);
        let lines = render_contract_diff(&diff);

        assert!(lines.iter().any(|line| {
            line.contains("CHANGED: TERM DeliveryPeriod") && line.contains("duration.amount")
        }));
        assert!(
            lines.iter()
                .any(|line| line.contains("ADDED: OBLIGATION AuditRights"))
        );
        assert!(
            lines.iter()
                .any(|line| line.contains("REMOVED: PHASE CureOrTerminate.Cure"))
        );
    }

    #[test]
    fn risk_warnings_flag_removed_terminal_remedy_and_removed_phase() {
        let old = contract_from_source(
            r#"contract C {
                obligation DeliverSoftware { action: "Deliver software" }
                remedy CureOrTerminate on breach_of(DeliverSoftware) {
                    phase Cure { action: "Cure" }
                }
            }"#,
        );
        let new = contract_from_source(
            r#"contract C {
                obligation DeliverSoftware { action: "Deliver software" }
            }"#,
        );

        let diff = diff_contracts(&old, &new);
        let warnings = generate_risk_warnings(&old, &new, &diff);

        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("breach of 'DeliverSoftware' now has no remedy"))
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.contains("removed remedy phase 'CureOrTerminate.Cure'"))
        );
    }
}
