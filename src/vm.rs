use std::collections::HashMap;

use chrono::NaiveDate;

use crate::chunk::{Chunk, OpCode, Value};
use crate::error::KontraError;
use crate::types::{
    ConditionExpr, ContractDef, DueDef, Duration, DurationUnit, EventDef, ObligationDef, Party,
    PhaseDef, RemedyDef, Term, TermAnchor,
};

pub struct VM {
    chunk: Chunk,
    ip: usize,
    stack: Vec<Value>,
    // Decision A: keep condition construction separate from runtime values.
    condition_stack: Vec<ConditionExpr>,
    contract: ContractDef,
    current_obligation: Option<ObligationDef>,
    current_remedy: Option<RemedyDef>,
    current_phase: Option<PhaseDef>,
}

impl VM {
    pub fn interpret(chunk: Chunk) -> Result<ContractDef, KontraError> {
        let mut vm = Self::new(chunk);
        vm.run()
    }

    fn new(chunk: Chunk) -> Self {
        Self {
            chunk,
            ip: 0,
            stack: Vec::new(),
            condition_stack: Vec::new(),
            contract: ContractDef {
                parties: HashMap::new(),
                events: HashMap::new(),
                terms: HashMap::new(),
                obligations: HashMap::new(),
                remedies: HashMap::new(),
            },
            current_obligation: None,
            current_remedy: None,
            current_phase: None,
        }
    }

    fn run(&mut self) -> Result<ContractDef, KontraError> {
        loop {
            let op_index = self.ip;
            let op = self.read_opcode(op_index)?;
            match op {
                OpCode::Constant => self.op_constant(op_index)?,
                OpCode::DefineParty => self.op_define_party(op_index)?,
                OpCode::DefineEvent => self.op_define_event(op_index)?,
                OpCode::DefineTerm => self.op_define_term(op_index)?,
                OpCode::BeginObligation => self.op_begin_obligation(op_index)?,
                OpCode::SetParty => self.op_set_party(op_index)?,
                OpCode::SetAction => self.op_set_action(op_index)?,
                OpCode::SetDue => self.op_set_due(op_index)?,
                OpCode::ConditionAfter => self.op_condition_after(op_index)?,
                OpCode::ConditionBefore => self.op_condition_before(op_index)?,
                OpCode::ConditionSatisfied => self.op_condition_satisfied(op_index)?,
                OpCode::ConditionOccurred => self.op_condition_occurred(op_index)?,
                OpCode::ConditionAnd => self.op_condition_and(op_index)?,
                OpCode::ConditionOr => self.op_condition_or(op_index)?,
                OpCode::SetCondition => self.op_set_condition(op_index)?,
                OpCode::EndObligation => self.op_end_obligation(op_index)?,
                OpCode::BeginRemedy => self.op_begin_remedy(op_index)?,
                OpCode::EndRemedy => self.op_end_remedy(op_index)?,
                OpCode::BeginPhase => self.op_begin_phase(op_index)?,
                OpCode::EndPhase => self.op_end_phase(op_index)?,
                OpCode::Return => {
                    if self.current_phase.is_some()
                        || self.current_remedy.is_some()
                        || self.current_obligation.is_some()
                    {
                        return Err(self.runtime_error(
                            op_index,
                            "Return reached while declaration scope is still open",
                        ));
                    }
                    return Ok(self.contract.clone());
                }
            }
        }
    }

    fn op_constant(&mut self, op_index: usize) -> Result<(), KontraError> {
        let idx = self.read_byte(op_index)? as usize;
        let value = self
            .chunk
            .constants
            .get(idx)
            .cloned()
            .ok_or_else(|| self.runtime_error(op_index, "Constant index out of bounds"))?;
        self.stack.push(value);
        Ok(())
    }

    fn op_define_party(&mut self, op_index: usize) -> Result<(), KontraError> {
        let name = self.pop_str(op_index, "DefineParty expects party name string on stack")?;
        let role = self.pop_identifier(op_index, "DefineParty expects role identifier on stack")?;
        if self.contract.parties.contains_key(&role) {
            return Err(self.runtime_error(
                op_index,
                &format!("Duplicate party role '{}'", role),
            ));
        }

        self.contract
            .parties
            .insert(role.clone(), Party { role, name });
        Ok(())
    }

    fn op_define_event(&mut self, op_index: usize) -> Result<(), KontraError> {
        let arg = self.pop_value(op_index)?;
        let name = self.pop_identifier(op_index, "DefineEvent expects event name identifier")?;
        if self.contract.events.contains_key(&name) {
            return Err(self.runtime_error(
                op_index,
                &format!("Duplicate event name '{}'", name),
            ));
        }

        let event = match arg {
            Value::Str(date_str) => {
                let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d").map_err(|_| {
                    self.runtime_error(op_index, "DefineEvent date must be YYYY-MM-DD")
                })?;
                EventDef::DateEvent(date)
            }
            Value::Identifier(role) => EventDef::TriggeredEvent(role),
            _ => {
                return Err(self.runtime_error(
                    op_index,
                    "DefineEvent expects date string or triggering party identifier",
                ));
            }
        };

        self.contract.events.insert(name, event);
        Ok(())
    }

    fn op_define_term(&mut self, op_index: usize) -> Result<(), KontraError> {
        let anchor_raw = self.pop_identifier(op_index, "DefineTerm expects anchor identifier")?;
        let unit_raw = self.pop_str(op_index, "DefineTerm expects duration unit string")?;
        let amount_raw = self.pop_num(op_index, "DefineTerm expects numeric duration amount")?;
        let name = self.pop_identifier(op_index, "DefineTerm expects term name identifier")?;
        if self.contract.terms.contains_key(&name) {
            return Err(self.runtime_error(
                op_index,
                &format!("Duplicate term name '{}'", name),
            ));
        }

        if amount_raw < 0.0 || amount_raw.fract() != 0.0 {
            return Err(self.runtime_error(
                op_index,
                "DefineTerm duration amount must be a non-negative integer",
            ));
        }
        let amount = amount_raw as u32;

        let unit = match unit_raw.as_str() {
            "calendar_days" => DurationUnit::CalendarDays,
            "business_days" => DurationUnit::BusinessDays,
            _ => return Err(self.runtime_error(op_index, "Unknown term duration unit")),
        };

        let anchor = match Self::parse_breach_of_wrapper(&anchor_raw) {
            Some(target) => TermAnchor::BreachOf(target),
            None => TermAnchor::Event(anchor_raw),
        };

        self.contract.terms.insert(
            name.clone(),
            Term {
                name,
                duration: Duration { amount, unit },
                anchor,
                calendar_ref: None,
            },
        );
        Ok(())
    }

    fn op_begin_obligation(&mut self, op_index: usize) -> Result<(), KontraError> {
        if self.current_obligation.is_some() {
            return Err(self.runtime_error(op_index, "Nested obligations are not supported"));
        }
        let name =
            self.pop_identifier(op_index, "BeginObligation expects obligation name identifier")?;
        self.current_obligation = Some(ObligationDef {
            name,
            party_role: None,
            action: None,
            due: None,
            condition: None,
        });
        Ok(())
    }

    fn op_end_obligation(&mut self, op_index: usize) -> Result<(), KontraError> {
        let obligation = self
            .current_obligation
            .take()
            .ok_or_else(|| self.runtime_error(op_index, "EndObligation without BeginObligation"))?;
        if self.contract.obligations.contains_key(&obligation.name) {
            return Err(self.runtime_error(
                op_index,
                &format!("Duplicate obligation name '{}'", obligation.name),
            ));
        }
        self.contract
            .obligations
            .insert(obligation.name.clone(), obligation);
        Ok(())
    }

    fn op_begin_remedy(&mut self, op_index: usize) -> Result<(), KontraError> {
        if self.current_remedy.is_some() {
            return Err(self.runtime_error(op_index, "Nested remedies are not supported"));
        }
        let breach_target =
            self.pop_identifier(op_index, "BeginRemedy expects breach target identifier")?;
        let name = self.pop_identifier(op_index, "BeginRemedy expects remedy name identifier")?;
        self.current_remedy = Some(RemedyDef {
            name,
            breach_target,
            party_role: None,
            action: None,
            due: None,
            condition: None,
            phases: Vec::new(),
        });
        Ok(())
    }

    fn op_end_remedy(&mut self, op_index: usize) -> Result<(), KontraError> {
        if self.current_phase.is_some() {
            return Err(self.runtime_error(
                op_index,
                "EndRemedy encountered while a phase is still open",
            ));
        }

        let remedy = self
            .current_remedy
            .take()
            .ok_or_else(|| self.runtime_error(op_index, "EndRemedy without BeginRemedy"))?;
        if self.contract.remedies.contains_key(&remedy.name) {
            return Err(self.runtime_error(
                op_index,
                &format!("Duplicate remedy name '{}'", remedy.name),
            ));
        }
        self.contract.remedies.insert(remedy.name.clone(), remedy);
        Ok(())
    }

    fn op_begin_phase(&mut self, op_index: usize) -> Result<(), KontraError> {
        if self.current_phase.is_some() {
            return Err(self.runtime_error(op_index, "Nested phases are not supported"));
        }
        if self.current_remedy.is_none() {
            return Err(self.runtime_error(
                op_index,
                "BeginPhase must occur inside a remedy",
            ));
        }

        let first = self.pop_identifier(op_index, "BeginPhase expects phase header constant")?;
        let (name, breach_target) = if let Some(target) = Self::parse_breach_of_wrapper(&first) {
            let phase_name = self.pop_identifier(
                op_index,
                "BeginPhase expected phase name before breach_of(...) marker",
            )?;
            (phase_name, Some(target))
        } else {
            (first, None)
        };

        self.current_phase = Some(PhaseDef {
            name,
            breach_target,
            party_role: None,
            action: None,
            due: None,
            condition: None,
        });
        Ok(())
    }

    fn op_end_phase(&mut self, op_index: usize) -> Result<(), KontraError> {
        let phase = self
            .current_phase
            .take()
            .ok_or_else(|| self.runtime_error(op_index, "EndPhase without BeginPhase"))?;
        if self.current_remedy.is_none() {
            return Err(self.runtime_error(op_index, "EndPhase must occur inside a remedy"));
        }
        let remedy = self.current_remedy.as_mut().expect("checked above");
        if remedy.phases.iter().any(|existing| existing.name == phase.name) {
            return Err(self.runtime_error(
                op_index,
                &format!("Duplicate phase name '{}'", phase.name),
            ));
        }
        remedy.phases.push(phase);
        Ok(())
    }

    fn op_set_party(&mut self, op_index: usize) -> Result<(), KontraError> {
        let value = self.pop_identifier(op_index, "SetParty expects role identifier")?;
        if let Some(phase) = self.current_phase.as_mut() {
            phase.party_role = Some(value);
            return Ok(());
        }
        if let Some(obligation) = self.current_obligation.as_mut() {
            obligation.party_role = Some(value);
            return Ok(());
        }
        if let Some(remedy) = self.current_remedy.as_mut() {
            remedy.party_role = Some(value);
            return Ok(());
        }
        Err(self.runtime_error(op_index, "SetParty used outside declaration scope"))
    }

    fn op_set_action(&mut self, op_index: usize) -> Result<(), KontraError> {
        let value = self.pop_str(op_index, "SetAction expects action string")?;
        if let Some(phase) = self.current_phase.as_mut() {
            phase.action = Some(value);
            return Ok(());
        }
        if let Some(obligation) = self.current_obligation.as_mut() {
            obligation.action = Some(value);
            return Ok(());
        }
        if let Some(remedy) = self.current_remedy.as_mut() {
            remedy.action = Some(value);
            return Ok(());
        }
        Err(self.runtime_error(op_index, "SetAction used outside declaration scope"))
    }

    fn op_set_due(&mut self, op_index: usize) -> Result<(), KontraError> {
        let raw_value = self.pop_value(op_index)?;
        let due = self.parse_due(raw_value, op_index)?;

        if let Some(phase) = self.current_phase.as_mut() {
            phase.due = Some(due);
            return Ok(());
        }
        if let Some(obligation) = self.current_obligation.as_mut() {
            obligation.due = Some(due);
            return Ok(());
        }
        if let Some(remedy) = self.current_remedy.as_mut() {
            remedy.due = Some(due);
            return Ok(());
        }
        Err(self.runtime_error(op_index, "SetDue used outside declaration scope"))
    }

    fn op_condition_after(&mut self, op_index: usize) -> Result<(), KontraError> {
        let arg = self.pop_identifier(op_index, "ConditionAfter expects identifier argument")?;
        self.condition_stack.push(ConditionExpr::After(arg));
        Ok(())
    }

    fn op_condition_before(&mut self, op_index: usize) -> Result<(), KontraError> {
        let arg = self.pop_identifier(op_index, "ConditionBefore expects identifier argument")?;
        self.condition_stack.push(ConditionExpr::Before(arg));
        Ok(())
    }

    fn op_condition_satisfied(&mut self, op_index: usize) -> Result<(), KontraError> {
        let arg =
            self.pop_identifier(op_index, "ConditionSatisfied expects identifier argument")?;
        self.condition_stack.push(ConditionExpr::Satisfied(arg));
        Ok(())
    }

    fn op_condition_occurred(&mut self, op_index: usize) -> Result<(), KontraError> {
        let arg = self.pop_identifier(op_index, "ConditionOccurred expects identifier argument")?;
        self.condition_stack.push(ConditionExpr::Occurred(arg));
        Ok(())
    }

    fn op_condition_and(&mut self, op_index: usize) -> Result<(), KontraError> {
        let right = self
            .condition_stack
            .pop()
            .ok_or_else(|| self.runtime_error(op_index, "ConditionAnd missing right operand"))?;
        let left = self
            .condition_stack
            .pop()
            .ok_or_else(|| self.runtime_error(op_index, "ConditionAnd missing left operand"))?;
        self.condition_stack
            .push(ConditionExpr::And(Box::new(left), Box::new(right)));
        Ok(())
    }

    fn op_condition_or(&mut self, op_index: usize) -> Result<(), KontraError> {
        let right = self
            .condition_stack
            .pop()
            .ok_or_else(|| self.runtime_error(op_index, "ConditionOr missing right operand"))?;
        let left = self
            .condition_stack
            .pop()
            .ok_or_else(|| self.runtime_error(op_index, "ConditionOr missing left operand"))?;
        self.condition_stack
            .push(ConditionExpr::Or(Box::new(left), Box::new(right)));
        Ok(())
    }

    fn op_set_condition(&mut self, op_index: usize) -> Result<(), KontraError> {
        let condition = self
            .condition_stack
            .pop()
            .ok_or_else(|| self.runtime_error(op_index, "SetCondition expects built condition"))?;

        if let Some(phase) = self.current_phase.as_mut() {
            phase.condition = Some(condition);
            return Ok(());
        }
        if let Some(obligation) = self.current_obligation.as_mut() {
            obligation.condition = Some(condition);
            return Ok(());
        }
        if let Some(remedy) = self.current_remedy.as_mut() {
            remedy.condition = Some(condition);
            return Ok(());
        }
        Err(self.runtime_error(op_index, "SetCondition used outside declaration scope"))
    }

    fn parse_due(&self, value: Value, op_index: usize) -> Result<DueDef, KontraError> {
        match value {
            Value::Identifier(term_ref) => Ok(DueDef::TermRef(term_ref)),
            Value::Str(inline) => self.parse_inline_due(&inline, op_index),
            _ => Err(self.runtime_error(op_index, "SetDue expects identifier or inline due string")),
        }
    }

    fn parse_inline_due(&self, raw: &str, op_index: usize) -> Result<DueDef, KontraError> {
        let parts: Vec<&str> = raw.split_whitespace().collect();
        if parts.len() != 4 || parts[2] != "from" {
            return Err(self.runtime_error(
                op_index,
                "Inline due must match: '<N> <calendar_days|business_days> from <Anchor>'",
            ));
        }

        let amount = parts[0]
            .parse::<u32>()
            .map_err(|_| self.runtime_error(op_index, "Inline due amount must be integer"))?;
        let unit = match parts[1] {
            "calendar_days" => DurationUnit::CalendarDays,
            "business_days" => DurationUnit::BusinessDays,
            _ => {
                return Err(self.runtime_error(
                    op_index,
                    "Inline due unit must be calendar_days or business_days",
                ));
            }
        };

        Ok(DueDef::InlineDuration {
            duration: Duration { amount, unit },
            anchor: parts[3].to_string(),
        })
    }

    fn pop_value(&mut self, op_index: usize) -> Result<Value, KontraError> {
        self.stack
            .pop()
            .ok_or_else(|| self.runtime_error(op_index, "Stack underflow"))
    }

    fn pop_identifier(&mut self, op_index: usize, context: &str) -> Result<String, KontraError> {
        match self.pop_value(op_index)? {
            Value::Identifier(s) => Ok(s),
            _ => Err(self.runtime_error(op_index, context)),
        }
    }

    fn pop_str(&mut self, op_index: usize, context: &str) -> Result<String, KontraError> {
        match self.pop_value(op_index)? {
            Value::Str(s) => Ok(s),
            _ => Err(self.runtime_error(op_index, context)),
        }
    }

    fn pop_num(&mut self, op_index: usize, context: &str) -> Result<f64, KontraError> {
        match self.pop_value(op_index)? {
            Value::Num(n) => Ok(n),
            _ => Err(self.runtime_error(op_index, context)),
        }
    }

    fn read_opcode(&mut self, op_index: usize) -> Result<OpCode, KontraError> {
        let byte = self.read_byte(op_index)?;
        match byte {
            0..=20 => Ok(OpCode::from(byte)),
            _ => Err(self.runtime_error(op_index, "Unknown opcode")),
        }
    }

    fn read_byte(&mut self, op_index: usize) -> Result<u8, KontraError> {
        let byte = self
            .chunk
            .code
            .get(self.ip)
            .copied()
            .ok_or_else(|| self.runtime_error(op_index, "Unexpected end of bytecode"))?;
        self.ip += 1;
        Ok(byte)
    }

    fn runtime_error(&self, op_index: usize, message: &str) -> KontraError {
        let line = self
            .chunk
            .lines
            .get(op_index)
            .copied()
            .or_else(|| self.chunk.lines.last().copied());
        KontraError::runtime(line, message.to_string())
    }

    fn parse_breach_of_wrapper(raw: &str) -> Option<String> {
        let prefix = "breach_of(";
        if raw.starts_with(prefix) && raw.ends_with(')') {
            let inner = &raw[prefix.len()..raw.len() - 1];
            if !inner.is_empty() {
                return Some(inner.to_string());
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{Chunk, OpCode, Value};
    use crate::compiler::Compiler;

    #[test]
    fn interpret_minimal_contract_builds_party_and_obligation() {
        let source = r#"contract Foo {
            parties { seller: "Acme Corp" }
            event Effective = date("2026-03-01")
            term DeliveryPeriod = 30 calendar_days from Effective
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software per schedule"
                due: DeliveryPeriod
                condition: after(Effective)
            }
        }"#;

        let chunk = Compiler::compile(source).expect("compile should succeed");
        let contract = VM::interpret(chunk).expect("vm interpret should succeed");

        let seller = contract
            .parties
            .get("seller")
            .expect("seller party should exist");
        assert_eq!(seller.name, "Acme Corp");

        let obligation = contract
            .obligations
            .get("DeliverSoftware")
            .expect("obligation should exist");
        assert_eq!(obligation.party_role.as_deref(), Some("seller"));
        assert_eq!(
            obligation.action.as_deref(),
            Some("Deliver software per schedule")
        );
        assert_eq!(
            obligation.due,
            Some(DueDef::TermRef("DeliveryPeriod".to_string()))
        );
        assert_eq!(
            obligation.condition,
            Some(ConditionExpr::After("Effective".to_string()))
        );
    }

    #[test]
    fn interpret_condition_and_builds_condition_tree() {
        let source = r#"contract Foo {
            obligation Pay {
                party: buyer
                action: "Pay fee"
                due: 15 calendar_days from AcceptanceNotice
                condition: satisfied(DeliverSoftware) and occurred(AcceptanceNotice)
            }
        }"#;

        let chunk = Compiler::compile(source).expect("compile should succeed");
        let contract = VM::interpret(chunk).expect("vm interpret should succeed");

        let obligation = contract.obligations.get("Pay").expect("obligation should exist");
        assert_eq!(
            obligation.due,
            Some(DueDef::InlineDuration {
                duration: Duration {
                    amount: 15,
                    unit: DurationUnit::CalendarDays,
                },
                anchor: "AcceptanceNotice".to_string(),
            })
        );
        assert_eq!(
            obligation.condition,
            Some(ConditionExpr::And(
                Box::new(ConditionExpr::Satisfied("DeliverSoftware".to_string())),
                Box::new(ConditionExpr::Occurred("AcceptanceNotice".to_string())),
            ))
        );
    }

    #[test]
    fn interpret_condition_grouping_with_or_and_before_builds_tree() {
        let source = r#"contract Foo {
            obligation Pay {
                party: buyer
                action: "Pay fee"
                due: 15 calendar_days from AcceptanceNotice
                condition: (satisfied(DeliverSoftware) or occurred(AcceptanceNotice)) and before(PaymentDeadline)
            }
        }"#;

        let chunk = Compiler::compile(source).expect("compile should succeed");
        let contract = VM::interpret(chunk).expect("vm interpret should succeed");

        let obligation = contract.obligations.get("Pay").expect("obligation should exist");
        assert_eq!(
            obligation.condition,
            Some(ConditionExpr::And(
                Box::new(ConditionExpr::Or(
                    Box::new(ConditionExpr::Satisfied("DeliverSoftware".to_string())),
                    Box::new(ConditionExpr::Occurred("AcceptanceNotice".to_string())),
                )),
                Box::new(ConditionExpr::Before("PaymentDeadline".to_string())),
            ))
        );
    }

    #[test]
    fn interpret_remedy_with_phase_builds_nested_structure() {
        let source = r#"contract Foo {
            remedy CureOrTerminate on breach_of(DeliverSoftware) {
                phase Cure {
                    party: licensor
                    action: "Deliver software in cure period"
                    due: CurePeriod
                    condition: after(Effective)
                }
            }
        }"#;

        let chunk = Compiler::compile(source).expect("compile should succeed");
        let contract = VM::interpret(chunk).expect("vm interpret should succeed");

        let remedy = contract
            .remedies
            .get("CureOrTerminate")
            .expect("remedy should exist");
        assert_eq!(remedy.breach_target, "DeliverSoftware");
        assert_eq!(remedy.phases.len(), 1);
        assert_eq!(remedy.phases[0].name, "Cure");
        assert_eq!(remedy.phases[0].party_role.as_deref(), Some("licensor"));
    }

    #[test]
    fn runtime_error_includes_source_line_for_stack_underflow() {
        let mut chunk = Chunk::new();
        chunk.write(u8::from(OpCode::DefineParty), 42);
        chunk.write(u8::from(OpCode::Return), 42);

        let result = VM::interpret(chunk);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("[line 42] Runtime error"),
            "expected runtime error with source line, got: {}",
            msg
        );
        assert!(
            msg.contains("DefineParty expects party name string on stack")
                || msg.contains("Stack underflow"),
            "expected actionable runtime error context, got: {}",
            msg
        );
    }

    #[test]
    fn vm_rejects_duplicate_party_roles_even_if_bytecode_repeats_them() {
        let mut chunk = Chunk::new();
        let idx_role_1 = chunk.add_constant(Value::Identifier("buyer".into()));
        let idx_name_1 = chunk.add_constant(Value::Str("Alice".into()));
        let idx_role_2 = chunk.add_constant(Value::Identifier("buyer".into()));
        let idx_name_2 = chunk.add_constant(Value::Str("Alicia".into()));

        chunk.write(u8::from(OpCode::Constant), 7);
        chunk.write(idx_role_1, 7);
        chunk.write(u8::from(OpCode::Constant), 7);
        chunk.write(idx_name_1, 7);
        chunk.write(u8::from(OpCode::DefineParty), 7);

        chunk.write(u8::from(OpCode::Constant), 8);
        chunk.write(idx_role_2, 8);
        chunk.write(u8::from(OpCode::Constant), 8);
        chunk.write(idx_name_2, 8);
        chunk.write(u8::from(OpCode::DefineParty), 8);
        chunk.write(u8::from(OpCode::Return), 8);

        let result = VM::interpret(chunk);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Duplicate party role 'buyer'"),
            "expected duplicate-party runtime error, got: {}",
            msg
        );
        assert!(
            msg.contains("[line 8] Runtime error"),
            "expected duplicate to report bytecode line, got: {}",
            msg
        );
    }
}
