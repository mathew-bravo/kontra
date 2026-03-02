use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;

use chrono::{Duration as ChronoDuration, NaiveDate};

use crate::calendar;
use crate::types::{
    CalendarRegistry, ConditionExpr, ContractDef, DueDef, Duration, DurationUnit, EventOccurrence,
    ObligationState, RemedyDef, TermAnchor,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObligationSnapshot {
    pub name: String,
    pub state: ObligationState,
    pub due_date: Option<NaiveDate>,
    pub days_overdue: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CascadeLink {
    pub from: String,
    pub to: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreachCascade {
    pub root: String,
    pub links: Vec<CascadeLink>,
}

impl fmt::Display for ObligationSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = Self::state_label(&self.state);

        match self.state {
            ObligationState::Pending => {
                write!(f, "{}: {} (blocked - condition not met)", self.name, state)
            }
            _ => match (self.due_date, self.days_overdue) {
                (Some(due), Some(days)) if days > 0 => {
                    let day_label = if days == 1 { "day" } else { "days" };
                    write!(
                        f,
                        "{}: {} (due {}, OVERDUE by {} {})",
                        self.name, state, due, days, day_label
                    )
                }
                (Some(due), _) => write!(f, "{}: {} (due {})", self.name, state, due),
                (None, _) => write!(f, "{}: {}", self.name, state),
            },
        }
    }
}

impl ObligationSnapshot {
    fn state_label(state: &ObligationState) -> &'static str {
        match state {
            ObligationState::Pending => "PENDING",
            ObligationState::Active => "ACTIVE",
            ObligationState::Satisfied => "SATISFIED",
            ObligationState::Breached => "BREACHED",
            ObligationState::Remedied => "REMEDIED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContractRuntime {
    pub contract: ContractDef,
    pub states: HashMap<String, ObligationState>,
    pub events: Vec<EventOccurrence>,
    pub current_date: Option<NaiveDate>,
    pub calendar_registry: CalendarRegistry,
    breach_dates: HashMap<String, NaiveDate>,
}

impl ContractRuntime {
    pub fn new(contract: ContractDef) -> Self {
        Self::with_calendar_registry(contract, CalendarRegistry::phase2_default())
            .expect("phase2 default calendar registry should always be valid")
    }

    pub fn with_calendar_registry(
        contract: ContractDef,
        registry: CalendarRegistry,
    ) -> Result<Self, String> {
        let calendar_registry = registry.normalize();
        Self::validate_term_calendar_refs(&contract, &calendar_registry)?;

        let mut states = HashMap::new();

        for name in contract.obligations.keys() {
            states.insert(name.clone(), ObligationState::Pending);
        }
        for remedy in contract.remedies.values() {
            // Direct remedy-body fields (if present) map to the remedy name.
            states
                .entry(remedy.name.clone())
                .or_insert(ObligationState::Pending);
            for phase in &remedy.phases {
                states
                    .entry(format!("{}.{}", remedy.name, phase.name))
                    .or_insert(ObligationState::Pending);
            }
        }

        Ok(Self {
            contract,
            states,
            events: Vec::new(),
            current_date: None,
            calendar_registry,
            breach_dates: HashMap::new(),
        })
    }

    pub fn trigger_event(&mut self, name: &str, date: NaiveDate) {
        self.events.push(EventOccurrence {
            name: name.to_string(),
            date,
        });
        self.evaluate_at(date);
    }

    pub fn fork(&self) -> Self {
        self.clone()
    }

    pub fn simulate_with(
        &self,
        hypothetical_events: &[(String, NaiveDate)],
        evaluate_at: Option<NaiveDate>,
    ) -> Self {
        let mut simulated = self.fork();
        simulated.apply_hypothetical_events(hypothetical_events, evaluate_at);
        simulated
    }

    pub fn apply_hypothetical_events(
        &mut self,
        hypothetical_events: &[(String, NaiveDate)],
        evaluate_at: Option<NaiveDate>,
    ) {
        let mut latest_trigger_date: Option<NaiveDate> = None;
        for (event_name, date) in hypothetical_events {
            self.trigger_event(event_name.as_str(), *date);
            latest_trigger_date = Some(match latest_trigger_date {
                Some(prev) if prev > *date => prev,
                _ => *date,
            });
        }

        if let Some(date) = evaluate_at.or(latest_trigger_date) {
            self.evaluate_at(date);
        }
    }

    pub fn trace_breach_cascade(&self, breached_item: &str) -> BreachCascade {
        let root = self.resolve_state_key(breached_item, None);
        let mut queue = VecDeque::from([root.clone()]);
        let mut expanded = BTreeSet::new();
        let mut seen_links = BTreeSet::new();
        let mut links = Vec::new();

        while let Some(source) = queue.pop_front() {
            if !expanded.insert(source.clone()) {
                continue;
            }

            for link in self.direct_cascade_links_from(&source) {
                let dedupe_key = format!("{}|{}|{}", link.from, link.to, link.reason);
                if !seen_links.insert(dedupe_key) {
                    continue;
                }
                if !expanded.contains(&link.to) {
                    queue.push_back(link.to.clone());
                }
                links.push(link);
            }
        }

        links.sort_by(|a, b| {
            a.from
                .cmp(&b.from)
                .then(a.to.cmp(&b.to))
                .then(a.reason.cmp(&b.reason))
        });

        BreachCascade { root, links }
    }

    pub fn satisfy(&mut self, name: &str) {
        if self.states.contains_key(name) {
            self.states.insert(name.to_string(), ObligationState::Satisfied);
            self.breach_dates.remove(name);
        }
    }

    pub fn query_state(&self) -> Vec<ObligationSnapshot> {
        let mut names: Vec<String> = self.states.keys().cloned().collect();
        names.sort();

        names
            .into_iter()
            .map(|name| {
                let state = self
                    .states
                    .get(&name)
                    .cloned()
                    .unwrap_or(ObligationState::Pending);
                let due_date = self.resolve_due_date_for_item(&name);
                let days_overdue = match (self.current_date, due_date, &state) {
                    (Some(current_date), Some(due), ObligationState::Active)
                    | (Some(current_date), Some(due), ObligationState::Breached)
                        if current_date > due =>
                    {
                        Some((current_date - due).num_days())
                    }
                    _ => None,
                };

                ObligationSnapshot {
                    name,
                    state,
                    due_date,
                    days_overdue,
                }
            })
            .collect()
    }

    pub fn evaluate_at(&mut self, date: NaiveDate) {
        self.current_date = Some(date);

        loop {
            let mut changed = false;

            let base_obligations: Vec<_> = self.contract.obligations.values().cloned().collect();
            for obligation in base_obligations {
                changed |= self.evaluate_runtime_item(
                    &obligation.name,
                    obligation.condition.as_ref(),
                    obligation.due.as_ref(),
                    None,
                    None,
                    date,
                );
            }

            let remedies: Vec<RemedyDef> = self.contract.remedies.values().cloned().collect();
            for remedy in remedies {
                changed |= self.evaluate_runtime_item(
                    &remedy.name,
                    remedy.condition.as_ref(),
                    remedy.due.as_ref(),
                    Some(remedy.breach_target.as_str()),
                    Some(remedy.name.as_str()),
                    date,
                );

                for phase in remedy.phases {
                    let phase_key = format!("{}.{}", remedy.name, phase.name);
                    let trigger_target = phase
                        .breach_target
                        .as_deref()
                        .unwrap_or(remedy.breach_target.as_str());

                    changed |= self.evaluate_runtime_item(
                        &phase_key,
                        phase.condition.as_ref(),
                        phase.due.as_ref(),
                        Some(trigger_target),
                        Some(remedy.name.as_str()),
                        date,
                    );
                }
            }

            if !changed {
                break;
            }
        }
    }

    fn evaluate_runtime_item(
        &mut self,
        item_name: &str,
        condition: Option<&ConditionExpr>,
        due: Option<&DueDef>,
        trigger_breach_of: Option<&str>,
        remedy_scope: Option<&str>,
        date: NaiveDate,
    ) -> bool {
        let mut changed = false;
        let state = self
            .states
            .get(item_name)
            .cloned()
            .unwrap_or(ObligationState::Pending);

        if matches!(state, ObligationState::Satisfied | ObligationState::Remedied) {
            return false;
        }

        if let Some(target) = trigger_breach_of
            && !self.is_breached(target, remedy_scope)
        {
            return false;
        }

        if state == ObligationState::Pending && self.is_condition_met(condition, remedy_scope, date) {
            self.states
                .insert(item_name.to_string(), ObligationState::Active);
            changed = true;
        }

        let state_after_activation = self
            .states
            .get(item_name)
            .cloned()
            .unwrap_or(ObligationState::Pending);
        if state_after_activation == ObligationState::Active
            && let Some(due_date) = self.resolve_due_date(due, remedy_scope)
            && date > due_date
        {
            self.states
                .insert(item_name.to_string(), ObligationState::Breached);
            let breach_date = due_date + ChronoDuration::days(1);
            self.breach_dates
                .entry(item_name.to_string())
                .or_insert(breach_date);
            changed = true;
        }

        changed
    }

    fn is_condition_met(
        &self,
        condition: Option<&ConditionExpr>,
        remedy_scope: Option<&str>,
        date: NaiveDate,
    ) -> bool {
        match condition {
            None => true,
            Some(expr) => self.evaluate_condition(expr, remedy_scope, date),
        }
    }

    fn evaluate_condition(
        &self,
        expr: &ConditionExpr,
        remedy_scope: Option<&str>,
        date: NaiveDate,
    ) -> bool {
        match expr {
            ConditionExpr::After(event_name) => self
                .first_event_date(event_name)
                .map(|event_date| date >= event_date)
                .unwrap_or(false),
            ConditionExpr::Before(event_name) => self
                .first_event_date(event_name)
                .map(|event_date| date < event_date)
                .unwrap_or(false),
            ConditionExpr::Satisfied(name) => self.is_satisfied(name, remedy_scope),
            ConditionExpr::Occurred(event_name) => self.first_event_date(event_name).is_some(),
            ConditionExpr::And(left, right) => {
                self.evaluate_condition(left, remedy_scope, date)
                    && self.evaluate_condition(right, remedy_scope, date)
            }
            ConditionExpr::Or(left, right) => {
                self.evaluate_condition(left, remedy_scope, date)
                    || self.evaluate_condition(right, remedy_scope, date)
            }
        }
    }

    fn resolve_due_date(&self, due: Option<&DueDef>, remedy_scope: Option<&str>) -> Option<NaiveDate> {
        match due {
            None => None,
            Some(DueDef::TermRef(term_name)) => self.resolve_term_due_date(term_name, remedy_scope),
            Some(DueDef::InlineDuration { duration, anchor }) => self
                .first_event_date(anchor)
                .map(|anchor_date| {
                    self.add_duration(anchor_date, duration, self.calendar_registry.default_calendar_id.as_str())
                }),
        }
    }

    fn resolve_due_date_for_item(&self, item_name: &str) -> Option<NaiveDate> {
        if let Some(obligation) = self.contract.obligations.get(item_name) {
            return self.resolve_due_date(obligation.due.as_ref(), None);
        }
        if let Some(remedy) = self.contract.remedies.get(item_name) {
            return self.resolve_due_date(remedy.due.as_ref(), Some(remedy.name.as_str()));
        }
        if let Some((remedy_name, phase_name)) = item_name.split_once('.')
            && let Some(remedy) = self.contract.remedies.get(remedy_name)
            && let Some(phase) = remedy.phases.iter().find(|phase| phase.name == phase_name)
        {
            return self.resolve_due_date(phase.due.as_ref(), Some(remedy.name.as_str()));
        }
        None
    }

    fn resolve_term_due_date(&self, term_name: &str, remedy_scope: Option<&str>) -> Option<NaiveDate> {
        let term = self.contract.terms.get(term_name)?;
        let calendar_id = self.selected_calendar_id_for_term(term_name)?;

        let anchor_date = match &term.anchor {
            TermAnchor::Event(event_name) => self.first_event_date(event_name),
            TermAnchor::BreachOf(target) => {
                let resolved = self.resolve_state_key(target, remedy_scope);
                self.breach_dates
                    .get(&resolved)
                    .copied()
                    .or_else(|| self.breach_dates.get(target).copied())
            }
        }?;

        Some(self.add_duration(anchor_date, &term.duration, calendar_id))
    }

    pub fn selected_calendar_id_for_term(&self, term_name: &str) -> Option<&str> {
        let term = self.contract.terms.get(term_name)?;
        match term.calendar_ref.as_deref() {
            Some(calendar_id) if self.calendar_registry.has_calendar(calendar_id) => Some(calendar_id),
            Some(_) => None,
            None => Some(self.calendar_registry.default_calendar_id.as_str()),
        }
    }

    fn first_event_date(&self, name: &str) -> Option<NaiveDate> {
        self.events
            .iter()
            .filter(|e| e.name == name)
            .map(|e| e.date)
            .min()
    }

    fn add_duration(&self, date: NaiveDate, duration: &Duration, calendar_id: &str) -> NaiveDate {
        match duration.unit {
            DurationUnit::CalendarDays => date + ChronoDuration::days(duration.amount as i64),
            DurationUnit::BusinessDays => self
                .calendar_for_id(calendar_id)
                .map(|calendar| calendar::add_business_days(date, duration.amount, calendar))
                .unwrap_or_else(|| date + ChronoDuration::days(duration.amount as i64)),
        }
    }

    fn calendar_for_id(&self, id: &str) -> Option<&crate::types::CalendarDef> {
        self.calendar_registry
            .calendars
            .get(id)
            .or_else(|| self.calendar_registry.default_calendar())
    }

    fn is_breached(&self, target: &str, remedy_scope: Option<&str>) -> bool {
        let resolved = self.resolve_state_key(target, remedy_scope);
        self.states
            .get(&resolved)
            .or_else(|| self.states.get(target))
            .map(|state| *state == ObligationState::Breached)
            .unwrap_or(false)
    }

    fn is_satisfied(&self, name: &str, remedy_scope: Option<&str>) -> bool {
        let resolved = self.resolve_state_key(name, remedy_scope);
        self.states
            .get(&resolved)
            .or_else(|| self.states.get(name))
            .map(|state| *state == ObligationState::Satisfied)
            .unwrap_or(false)
    }

    fn resolve_state_key(&self, name: &str, remedy_scope: Option<&str>) -> String {
        if self.states.contains_key(name) {
            return name.to_string();
        }
        if let Some(scope) = remedy_scope {
            let scoped = format!("{}.{}", scope, name);
            if self.states.contains_key(&scoped) {
                return scoped;
            }
        }
        name.to_string()
    }

    fn direct_cascade_links_from(&self, source: &str) -> Vec<CascadeLink> {
        let mut links = Vec::new();

        for remedy in self.contract.remedies.values() {
            if self.matches_dependency_target(source, remedy.breach_target.as_str(), None) {
                links.push(CascadeLink {
                    from: source.to_string(),
                    to: remedy.name.clone(),
                    reason: "remedy breach target".to_string(),
                });
            }

            if self.condition_depends_on_satisfied(
                remedy.condition.as_ref(),
                source,
                Some(remedy.name.as_str()),
            ) {
                links.push(CascadeLink {
                    from: source.to_string(),
                    to: remedy.name.clone(),
                    reason: "remedy condition depends on satisfied(...)".to_string(),
                });
            }

            for phase in &remedy.phases {
                let phase_key = format!("{}.{}", remedy.name, phase.name);
                let trigger = phase
                    .breach_target
                    .as_deref()
                    .unwrap_or(remedy.breach_target.as_str());
                if self.matches_dependency_target(source, trigger, Some(remedy.name.as_str())) {
                    links.push(CascadeLink {
                        from: source.to_string(),
                        to: phase_key.clone(),
                        reason: "phase breach trigger".to_string(),
                    });
                }

                if self.condition_depends_on_satisfied(
                    phase.condition.as_ref(),
                    source,
                    Some(remedy.name.as_str()),
                ) {
                    links.push(CascadeLink {
                        from: source.to_string(),
                        to: phase_key,
                        reason: "phase condition depends on satisfied(...)".to_string(),
                    });
                }
            }
        }

        for obligation in self.contract.obligations.values() {
            if self.condition_depends_on_satisfied(obligation.condition.as_ref(), source, None) {
                links.push(CascadeLink {
                    from: source.to_string(),
                    to: obligation.name.clone(),
                    reason: "obligation condition depends on satisfied(...)".to_string(),
                });
            }
        }

        for (term_name, target) in self.term_breach_anchors() {
            for (item, scope) in self.items_due_on_term(term_name.as_str()) {
                if !self.matches_dependency_target(source, target.as_str(), scope.as_deref()) {
                    continue;
                }
                links.push(CascadeLink {
                    from: source.to_string(),
                    to: item,
                    reason: format!("due date anchored by term '{}'", term_name),
                });
            }
        }

        links
    }

    fn items_due_on_term(&self, term_name: &str) -> Vec<(String, Option<String>)> {
        let mut items = Vec::new();
        for obligation in self.contract.obligations.values() {
            if obligation_uses_term(obligation.due.as_ref(), term_name) {
                items.push((obligation.name.clone(), None));
            }
        }
        for remedy in self.contract.remedies.values() {
            if obligation_uses_term(remedy.due.as_ref(), term_name) {
                items.push((remedy.name.clone(), None));
            }
            for phase in &remedy.phases {
                if obligation_uses_term(phase.due.as_ref(), term_name) {
                    items.push((
                        format!("{}.{}", remedy.name, phase.name),
                        Some(remedy.name.clone()),
                    ));
                }
            }
        }
        items.sort_by(|a, b| a.0.cmp(&b.0));
        items
    }

    fn term_breach_anchors(&self) -> Vec<(String, String)> {
        let mut anchors = Vec::new();
        for term in self.contract.terms.values() {
            if let TermAnchor::BreachOf(target) = &term.anchor {
                anchors.push((term.name.clone(), target.clone()));
            }
        }
        anchors.sort_by(|a, b| a.0.cmp(&b.0));
        anchors
    }

    fn condition_depends_on_satisfied(
        &self,
        condition: Option<&ConditionExpr>,
        source: &str,
        remedy_scope: Option<&str>,
    ) -> bool {
        let Some(condition) = condition else {
            return false;
        };
        match condition {
            ConditionExpr::Satisfied(target) => {
                self.matches_dependency_target(source, target.as_str(), remedy_scope)
            }
            ConditionExpr::And(left, right) | ConditionExpr::Or(left, right) => {
                self.condition_depends_on_satisfied(Some(left.as_ref()), source, remedy_scope)
                    || self.condition_depends_on_satisfied(Some(right.as_ref()), source, remedy_scope)
            }
            _ => false,
        }
    }

    fn matches_dependency_target(
        &self,
        source: &str,
        target: &str,
        remedy_scope: Option<&str>,
    ) -> bool {
        source == self.resolve_state_key(target, remedy_scope)
    }

    fn validate_term_calendar_refs(
        contract: &ContractDef,
        calendar_registry: &CalendarRegistry,
    ) -> Result<(), String> {
        for term in contract.terms.values() {
            if let Some(calendar_id) = term.calendar_ref.as_deref()
                && !calendar_registry.has_calendar(calendar_id)
            {
                return Err(format!(
                    "Unknown calendar '{}' referenced by term '{}'",
                    calendar_id, term.name
                ));
            }
        }
        Ok(())
    }
}

fn obligation_uses_term(due: Option<&DueDef>, term_name: &str) -> bool {
    matches!(due, Some(DueDef::TermRef(name)) if name == term_name)
}

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;

    use crate::compiler::Compiler;
    use crate::types::{CalendarDef, CalendarRegistry};
    use crate::vm::VM;

    use super::*;

    fn runtime_from_source(source: &str) -> ContractRuntime {
        let chunk = Compiler::compile(source).expect("compile should succeed");
        let contract = VM::interpret(chunk).expect("vm interpret should succeed");
        ContractRuntime::new(contract)
    }

    fn contract_from_source(source: &str) -> ContractDef {
        let chunk = Compiler::compile(source).expect("compile should succeed");
        VM::interpret(chunk).expect("vm interpret should succeed")
    }

    fn snapshot_due_date(runtime: &ContractRuntime, name: &str) -> Option<NaiveDate> {
        runtime
            .query_state()
            .into_iter()
            .find(|snapshot| snapshot.name == name)
            .and_then(|snapshot| snapshot.due_date)
    }

    #[test]
    fn runtime_uses_default_calendar_for_term_without_override() {
        let source = r#"contract Foo {
            term DeliveryWindow = 5 business_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
            }
        }"#;

        let runtime = runtime_from_source(source);
        assert_eq!(
            runtime.selected_calendar_id_for_term("DeliveryWindow"),
            Some(CalendarRegistry::PHASE2_DEFAULT_CALENDAR_ID),
        );
    }

    #[test]
    fn runtime_uses_term_calendar_override_when_present() {
        let source = r#"contract Foo {
            term DeliveryWindow = 5 business_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
            }
        }"#;

        let mut contract = contract_from_source(source);
        contract
            .terms
            .get_mut("DeliveryWindow")
            .expect("term should exist")
            .calendar_ref = Some("us_ny".to_string());

        let registry = CalendarRegistry::from_calendars(
            "default",
            vec![CalendarDef::standard("default"), CalendarDef::standard("us_ny")],
        )
        .expect("registry should be valid");
        let runtime = ContractRuntime::with_calendar_registry(contract, registry)
            .expect("runtime should accept known calendar override");

        assert_eq!(
            runtime.selected_calendar_id_for_term("DeliveryWindow"),
            Some("us_ny"),
        );
    }

    #[test]
    fn runtime_rejects_unknown_term_calendar_override() {
        let source = r#"contract Foo {
            term DeliveryWindow = 5 business_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
            }
        }"#;

        let mut contract = contract_from_source(source);
        contract
            .terms
            .get_mut("DeliveryWindow")
            .expect("term should exist")
            .calendar_ref = Some("unknown_calendar".to_string());

        let err =
            ContractRuntime::with_calendar_registry(contract, CalendarRegistry::phase2_default())
                .expect_err("unknown term calendar should be rejected");
        assert!(
            err.contains("Unknown calendar 'unknown_calendar' referenced by term 'DeliveryWindow'")
        );
    }

    #[test]
    fn business_day_term_skips_weekends_in_due_resolution() {
        let source = r#"contract Foo {
            term DeliveryWindow = 1 business_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
                condition: after(Effective)
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.trigger_event(
            "Effective",
            NaiveDate::from_ymd_opt(2026, 3, 6).expect("valid date"), // Friday
        );
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 6).expect("valid date"));

        assert_eq!(
            snapshot_due_date(&runtime, "DeliverSoftware"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 9).expect("valid date")), // Monday
        );
    }

    #[test]
    fn business_day_term_skips_configured_holidays_in_due_resolution() {
        let source = r#"contract Foo {
            term DeliveryWindow = 1 business_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
                condition: after(Effective)
            }
        }"#;

        let mut contract = contract_from_source(source);
        contract
            .terms
            .get_mut("DeliveryWindow")
            .expect("term should exist")
            .calendar_ref = Some("us_ny".to_string());

        let mut holiday_calendar = CalendarDef::standard("us_ny");
        holiday_calendar.jurisdiction = Some("US-NY".to_string());
        holiday_calendar
            .holidays
            .insert(NaiveDate::from_ymd_opt(2026, 3, 9).expect("valid date")); // Monday holiday

        let registry =
            CalendarRegistry::from_calendars("default", vec![CalendarDef::standard("default"), holiday_calendar])
                .expect("registry should be valid");
        let mut runtime = ContractRuntime::with_calendar_registry(contract, registry)
            .expect("runtime should accept configured calendar");

        runtime.trigger_event(
            "Effective",
            NaiveDate::from_ymd_opt(2026, 3, 6).expect("valid date"), // Friday
        );
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 6).expect("valid date"));

        assert_eq!(
            snapshot_due_date(&runtime, "DeliverSoftware"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 10).expect("valid date")), // Tuesday
        );
    }

    #[test]
    fn business_and_calendar_day_due_dates_diverge() {
        let source = r#"contract Foo {
            term BizWindow = 1 business_days from Effective
            term CalWindow = 1 calendar_days from Effective

            obligation DeliverBusiness {
                party: seller
                action: "Deliver with business-day term"
                due: BizWindow
                condition: after(Effective)
            }

            obligation DeliverCalendar {
                party: seller
                action: "Deliver with calendar-day term"
                due: CalWindow
                condition: after(Effective)
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.trigger_event(
            "Effective",
            NaiveDate::from_ymd_opt(2026, 3, 6).expect("valid date"), // Friday
        );
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 6).expect("valid date"));

        assert_eq!(
            snapshot_due_date(&runtime, "DeliverBusiness"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 9).expect("valid date")), // Monday
        );
        assert_eq!(
            snapshot_due_date(&runtime, "DeliverCalendar"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 7).expect("valid date")), // Saturday
        );
    }

    #[test]
    fn condition_precedence_treats_and_tighter_than_or() {
        let source = r#"contract Foo {
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
            }

            obligation PayLicenseFee {
                party: buyer
                action: "Pay fee"
                condition: satisfied(DeliverSoftware) or occurred(AcceptanceNotice) and occurred(ApprovalNotice)
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.satisfy("DeliverSoftware");
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 5).expect("valid date"));

        assert_eq!(
            runtime.states.get("PayLicenseFee"),
            Some(&ObligationState::Active),
            "expected active because expression should parse as A or (B and C)",
        );
    }

    #[test]
    fn condition_grouping_changes_default_precedence() {
        let source = r#"contract Foo {
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
            }

            obligation PayLicenseFee {
                party: buyer
                action: "Pay fee"
                condition: (satisfied(DeliverSoftware) or occurred(AcceptanceNotice)) and occurred(ApprovalNotice)
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.satisfy("DeliverSoftware");
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 5).expect("valid date"));

        assert_eq!(
            runtime.states.get("PayLicenseFee"),
            Some(&ObligationState::Pending),
            "grouping should require occurred(ApprovalNotice)",
        );
    }

    #[test]
    fn condition_before_uses_event_date_threshold() {
        let source = r#"contract Foo {
            obligation EarlyAction {
                party: seller
                action: "Act before cutoff"
                condition: before(Cutoff)
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.events.push(EventOccurrence {
            name: "Cutoff".to_string(),
            date: NaiveDate::from_ymd_opt(2026, 3, 10).expect("valid date"),
        });
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 5).expect("valid date"));

        assert_eq!(
            runtime.states.get("EarlyAction"),
            Some(&ObligationState::Active),
            "before(Cutoff) should be true while eval date is earlier than cutoff date",
        );
    }

    #[test]
    fn pending_before_condition_is_met() {
        let source = r#"contract Foo {
            event Effective = date("2026-03-01")
            term PayWindow = 5 calendar_days from Effective
            obligation PayFee {
                party: buyer
                action: "Pay fee"
                due: PayWindow
                condition: occurred(AcceptanceNotice)
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 10).expect("valid date"));

        assert_eq!(
            runtime.states.get("PayFee"),
            Some(&ObligationState::Pending),
            "obligation should remain pending before AcceptanceNotice occurs",
        );
    }

    #[test]
    fn obligation_without_condition_activates_when_evaluated() {
        let source = r#"contract Foo {
            obligation Standalone {
                party: seller
                action: "Do the thing"
                due: 5 calendar_days from Effective
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 10).expect("valid date"));

        assert_eq!(
            runtime.states.get("Standalone"),
            Some(&ObligationState::Active),
            "obligation with no condition should activate during evaluation",
        );
    }

    #[test]
    fn active_when_condition_becomes_true() {
        let source = r#"contract Foo {
            term DeliveryWindow = 10 calendar_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
                condition: after(Effective)
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.trigger_event(
            "Effective",
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date"),
        );

        assert_eq!(
            runtime.states.get("DeliverSoftware"),
            Some(&ObligationState::Active),
            "obligation should activate after Effective occurs",
        );
    }

    #[test]
    fn active_obligation_breaches_after_deadline() {
        let source = r#"contract Foo {
            term DeliveryWindow = 1 calendar_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
                condition: after(Effective)
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.trigger_event(
            "Effective",
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date"),
        );
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 3).expect("valid date"));

        assert_eq!(
            runtime.states.get("DeliverSoftware"),
            Some(&ObligationState::Breached),
            "obligation should breach once evaluation date exceeds due date",
        );
    }

    #[test]
    fn remedy_phase_activates_when_target_breaches() {
        let source = r#"contract Foo {
            term DeliveryWindow = 1 calendar_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
                condition: after(Effective)
            }

            remedy CureOrTerminate on breach_of(DeliverSoftware) {
                phase Cure {
                    party: seller
                    action: "Deliver software in cure period"
                    due: 10 calendar_days from Effective
                    condition: after(Effective)
                }
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.trigger_event(
            "Effective",
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date"),
        );
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 3).expect("valid date"));

        assert_eq!(
            runtime.states.get("DeliverSoftware"),
            Some(&ObligationState::Breached),
            "base obligation should breach first",
        );
        assert_eq!(
            runtime.states.get("CureOrTerminate.Cure"),
            Some(&ObligationState::Active),
            "phase obligation should activate after breach trigger",
        );
    }

    #[test]
    fn breach_anchor_uses_first_day_after_due_not_eval_date() {
        let source = r#"contract Foo {
            term DeliveryWindow = 1 calendar_days from Effective
            term CurePeriod = 10 calendar_days from breach_of(DeliverSoftware)

            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
                condition: after(Effective)
            }

            remedy CureOrTerminate on breach_of(DeliverSoftware) {
                phase Cure {
                    party: seller
                    action: "Deliver software in cure period"
                    due: CurePeriod
                    condition: after(Effective)
                }
                phase Terminate on breach_of(Cure) {
                    action: "Terminate contract"
                }
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.trigger_event(
            "Effective",
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date"),
        );
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 4, 30).expect("valid date"));

        // DeliverSoftware due: 2026-03-02 -> breach date should lock to 2026-03-03.
        // CurePeriod (10 days) should therefore expire on 2026-03-13, so Cure is breached
        // and Terminate becomes active by 2026-04-30.
        assert_eq!(
            runtime.states.get("DeliverSoftware"),
            Some(&ObligationState::Breached)
        );
        assert_eq!(
            runtime.states.get("CureOrTerminate.Cure"),
            Some(&ObligationState::Breached)
        );
        assert_eq!(
            runtime.states.get("CureOrTerminate.Terminate"),
            Some(&ObligationState::Active)
        );
    }

    #[test]
    fn query_state_includes_all_runtime_items_sorted() {
        let source = r#"contract Foo {
            term DeliveryWindow = 1 calendar_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
                condition: after(Effective)
            }

            remedy CureOrTerminate on breach_of(DeliverSoftware) {
                phase Cure {
                    party: seller
                    action: "Deliver software in cure period"
                    due: 10 calendar_days from Effective
                    condition: after(Effective)
                }
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.trigger_event(
            "Effective",
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date"),
        );
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 3).expect("valid date"));

        let snapshots = runtime.query_state();
        let names: Vec<String> = snapshots.into_iter().map(|s| s.name).collect();
        assert_eq!(
            names,
            vec![
                "CureOrTerminate".to_string(),
                "CureOrTerminate.Cure".to_string(),
                "DeliverSoftware".to_string(),
            ],
            "query_state should include base + remedy + phase and sort names lexicographically",
        );
    }

    #[test]
    fn query_state_computes_due_and_overdue_for_breached_item() {
        let source = r#"contract Foo {
            term DeliveryWindow = 1 calendar_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
                condition: after(Effective)
            }
        }"#;

        let mut runtime = runtime_from_source(source);
        runtime.trigger_event(
            "Effective",
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date"),
        );
        runtime.evaluate_at(NaiveDate::from_ymd_opt(2026, 3, 5).expect("valid date"));

        let deliver = runtime
            .query_state()
            .into_iter()
            .find(|snapshot| snapshot.name == "DeliverSoftware")
            .expect("DeliverSoftware snapshot should exist");

        assert_eq!(deliver.state, ObligationState::Breached);
        assert_eq!(
            deliver.due_date,
            Some(NaiveDate::from_ymd_opt(2026, 3, 2).expect("valid date"))
        );
        assert_eq!(deliver.days_overdue, Some(3));
    }

    #[test]
    fn simulate_with_does_not_mutate_canonical_runtime() {
        let source = r#"contract Foo {
            term DeliveryWindow = 1 calendar_days from Effective
            obligation Deliver {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
                condition: after(Effective)
            }
        }"#;

        let runtime = runtime_from_source(source);
        let hypothetical_events = vec![(
            "Effective".to_string(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date"),
        )];

        let simulated = runtime.simulate_with(
            &hypothetical_events,
            Some(NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date")),
        );

        assert!(runtime.events.is_empty(), "canonical runtime must remain untouched");
        assert_eq!(runtime.current_date, None);
        assert_eq!(
            runtime.states.get("Deliver"),
            Some(&ObligationState::Pending),
            "canonical state should remain pending",
        );

        assert_eq!(simulated.events.len(), 1, "simulated runtime should include event");
        assert_eq!(
            simulated.states.get("Deliver"),
            Some(&ObligationState::Active),
            "simulated state should evaluate independently",
        );
    }

    #[test]
    fn forked_runtime_can_diverge_from_canonical_state() {
        let source = r#"contract Foo {
            obligation Deliver {
                party: seller
                action: "Deliver software"
            }
        }"#;

        let canonical = runtime_from_source(source);
        let mut fork = canonical.fork();
        fork.satisfy("Deliver");

        assert_eq!(
            canonical.states.get("Deliver"),
            Some(&ObligationState::Pending),
            "canonical should stay unchanged after fork mutation",
        );
        assert_eq!(
            fork.states.get("Deliver"),
            Some(&ObligationState::Satisfied),
            "fork should mutate independently",
        );
    }

    #[test]
    fn repeated_simulations_from_same_base_are_deterministic() {
        let source = r#"contract Foo {
            term DeliveryWindow = 1 calendar_days from Effective
            obligation Deliver {
                party: seller
                action: "Deliver software"
                due: DeliveryWindow
                condition: after(Effective)
            }
        }"#;

        let runtime = runtime_from_source(source);
        let hypothetical_events = vec![(
            "Effective".to_string(),
            NaiveDate::from_ymd_opt(2026, 3, 1).expect("valid date"),
        )];

        let sim_a = runtime.simulate_with(&hypothetical_events, None);
        let sim_b = runtime.simulate_with(&hypothetical_events, None);

        assert_eq!(sim_a.events, sim_b.events);
        assert_eq!(sim_a.current_date, sim_b.current_date);
        assert_eq!(sim_a.query_state(), sim_b.query_state());
    }

    #[test]
    fn breach_cascade_traces_multi_step_and_scoped_dependencies() {
        let source = r#"contract Foo {
            term CureWindow = 5 calendar_days from breach_of(Cure)

            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
            }

            remedy CureOrTerminate on breach_of(DeliverSoftware) {
                phase Cure {
                    action: "Cure delivery"
                }
                phase Notice {
                    action: "Issue notice"
                    due: CureWindow
                    condition: satisfied(Cure)
                }
                phase Terminate on breach_of(Cure) {
                    action: "Terminate contract"
                }
            }
        }"#;

        let runtime = runtime_from_source(source);
        let cascade = runtime.trace_breach_cascade("DeliverSoftware");

        assert!(
            cascade.links.iter().any(|link| {
                link.from == "DeliverSoftware"
                    && link.to == "CureOrTerminate.Cure"
                    && link.reason.contains("phase breach trigger")
            }),
            "breach should trigger cure phase",
        );
        assert!(
            cascade.links.iter().any(|link| {
                link.from == "CureOrTerminate.Cure"
                    && link.to == "CureOrTerminate.Terminate"
                    && link.reason.contains("phase breach trigger")
            }),
            "cure breach should trigger terminate phase via scoped breach_of(Cure)",
        );
        assert!(
            cascade.links.iter().any(|link| {
                link.from == "CureOrTerminate.Cure"
                    && link.to == "CureOrTerminate.Notice"
                    && (link.reason.contains("condition depends on satisfied")
                        || link.reason.contains("due date anchored by term"))
            }),
            "cure breach should impact notice phase via scoped dependency",
        );
    }

    #[test]
    fn breach_cascade_is_deterministic_across_repeated_runs() {
        let source = r#"contract Foo {
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
            }

            remedy CureOrTerminate on breach_of(DeliverSoftware) {
                phase Cure {
                    action: "Cure delivery"
                }
            }
        }"#;

        let runtime = runtime_from_source(source);
        let first = runtime.trace_breach_cascade("DeliverSoftware");
        let second = runtime.trace_breach_cascade("DeliverSoftware");
        assert_eq!(first, second);
    }

    #[test]
    fn snapshot_display_formats_pending_and_breached() {
        let pending = ObligationSnapshot {
            name: "PayLicenseFee".to_string(),
            state: ObligationState::Pending,
            due_date: None,
            days_overdue: None,
        };
        assert_eq!(
            pending.to_string(),
            "PayLicenseFee: PENDING (blocked - condition not met)"
        );

        let breached = ObligationSnapshot {
            name: "DeliverSoftware".to_string(),
            state: ObligationState::Breached,
            due_date: Some(NaiveDate::from_ymd_opt(2026, 4, 14).expect("valid date")),
            days_overdue: Some(1),
        };
        assert_eq!(
            breached.to_string(),
            "DeliverSoftware: BREACHED (due 2026-04-14, OVERDUE by 1 day)"
        );
    }
}
