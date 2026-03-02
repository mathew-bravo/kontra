use std::collections::HashSet;

use crate::chunk::{Chunk, OpCode, Value};
use crate::error::KontraError;
use crate::error::Span;
use crate::scanner::Scanner;
use crate::token::{Token, TokenType};

// ---------------------------------------------------------------------------
// Compiler
// ---------------------------------------------------------------------------

pub struct Compiler {
    scanner: Scanner,
    current: Token,
    previous: Token,
    chunk: Chunk,
    had_error: bool,
    panic_mode: bool,
    errors: Vec<KontraError>,
    party_roles: HashSet<String>,
    event_names: HashSet<String>,
    term_names: HashSet<String>,
    obligation_names: HashSet<String>,
    remedy_names: HashSet<String>,
}

/// Create a zero-position sentinel token used to initialise `current`/`previous`
/// before the first call to `advance`.
fn sentinel_token() -> Token {
    Token {
        token_type: TokenType::Eof,
        lexeme: String::new(),
        span: Span::new(0, 0, 0, 0),
    }
}

impl Compiler {
    // ── construction ────────────────────────────────────────────────

    fn new(source: &str) -> Self {
        Self {
            scanner: Scanner::new(source),
            current: sentinel_token(),
            previous: sentinel_token(),
            chunk: Chunk::new(),
            had_error: false,
            panic_mode: false,
            errors: Vec::new(),
            party_roles: HashSet::new(),
            event_names: HashSet::new(),
            term_names: HashSet::new(),
            obligation_names: HashSet::new(),
            remedy_names: HashSet::new(),
        }
    }

    // ── public entry point ──────────────────────────────────────────

    /// Compile a source string into a bytecode `Chunk`.
    ///
    /// Returns `Err` with the first error encountered if compilation fails.
    pub fn compile(source: &str) -> Result<Chunk, KontraError> {
        let mut compiler = Compiler::new(source);
        compiler.advance();

        // If the source is non-empty, parse the top-level contract wrapper.
        if !compiler.check(TokenType::Eof) {
            compiler.contract();
        }

        compiler.emit_return();

        if compiler.had_error {
            Err(compiler.errors.into_iter().next().unwrap())
        } else {
            Ok(compiler.chunk)
        }
    }

    // ── token navigation ────────────────────────────────────────────

    /// Consume the next token from the scanner.
    ///
    /// Skips (and reports) any scanner-level error tokens so that
    /// `self.current` always holds a non-error token afterwards.
    fn advance(&mut self) {
        self.previous = self.current.clone();

        loop {
            self.current = self.scanner.scan_token();
            if self.current.token_type != TokenType::Error {
                break;
            }
            // The scanner stuffs the error message into `lexeme`.
            let msg = self.current.lexeme.clone();
            self.error_at_current(&msg);
        }
    }

    /// Advance if the current token matches `expected`, otherwise report
    /// an error with `msg`.
    fn consume(&mut self, expected: TokenType, msg: &str) {
        if self.current.token_type == expected {
            self.advance();
            return;
        }
        self.error_at_current(msg);
    }

    /// Returns `true` if the current token is of type `tt`.
    fn check(&self, tt: TokenType) -> bool {
        self.current.token_type == tt
    }

    /// If the current token matches `tt`, consume it and return `true`.
    /// Otherwise leave the token stream untouched and return `false`.
    fn match_token(&mut self, tt: TokenType) -> bool {
        if !self.check(tt) {
            return false;
        }
        self.advance();
        true
    }

    // ── parsing ──────────────────────────────────────────────────────

    /// Parse the top-level `contract Name { … }` wrapper.
    fn contract(&mut self) {
        self.consume(TokenType::Contract, "Expected 'contract'");
        self.consume(TokenType::Identifier, "Expected contract name");
        // Contract name is consumed but not emitted — no opcode for it yet.
        self.consume(TokenType::LeftBrace, "Expected '{' after contract name");

        while !self.check(TokenType::RightBrace) && !self.check(TokenType::Eof) {
            self.declaration();
        }

        self.consume(TokenType::RightBrace, "Expected '}' after contract body");
    }

    /// Dispatch to the appropriate declaration parser based on the current
    /// keyword token. Resets `panic_mode` after each declaration so that
    /// one bad declaration doesn't suppress errors in the next.
    fn declaration(&mut self) {
        match self.current.token_type {
            TokenType::Parties => self.parties_decl(),
            TokenType::Event => self.event_decl(),
            TokenType::Term => self.term_decl(),
            TokenType::Obligation => self.obligation_decl(),
            TokenType::Remedy => self.remedy_decl(),

            _ => {
                self.error_at_current("Expected declaration (parties, event, term, obligation, or remedy)");
                // Skip the unexpected token to avoid infinite loop.
                self.advance();
            }
        }

        // Reset panic mode so errors in subsequent declarations are reported.
        self.panic_mode = false;
    }

    /// Parse `parties { role: "name", ... }`.
    ///
    /// For each pair, emits:
    ///   `Constant(role_identifier)`, `Constant(name_string)`, `DefineParty`
    fn parties_decl(&mut self) {
        self.consume(TokenType::Parties, "Expected 'parties'");
        self.consume(TokenType::LeftBrace, "Expected '{' after 'parties'");

        while !self.check(TokenType::RightBrace) && !self.check(TokenType::Eof) {
            // role identifier
            self.consume(TokenType::Identifier, "Expected party role");
            let role = self.previous.lexeme.clone();
            if !self.party_roles.insert(role.clone()) {
                self.error_at_previous(&format!("Duplicate party role '{}'", role));
            }

            self.consume(TokenType::Colon, "Expected ':' after party role");

            // party name string
            self.consume(TokenType::StringLiteral, "Expected party name string");
            // Strip surrounding quotes from the lexeme (scanner keeps them).
            let raw = &self.previous.lexeme;
            let name = raw[1..raw.len() - 1].to_string();

            // Emit bytecode: Constant(role), Constant(name), DefineParty
            self.emit_constant(Value::Identifier(role));
            self.emit_constant(Value::Str(name));
            self.emit_byte(u8::from(OpCode::DefineParty));
        }

        self.consume(TokenType::RightBrace, "Expected '}' after parties block");
    }

    /// Parse:
    ///   event NAME = date("YYYY-MM-DD")
    ///   event NAME = triggered_by(PARTY_ROLE)
    ///
    /// Emits:
    ///   Constant(event_name), Constant(arg), DefineEvent
    fn event_decl(&mut self) {
        self.consume(TokenType::Event, "Expected 'event'");
        self.consume(TokenType::Identifier, "Expected event name");
        let event_name = self.previous.lexeme.clone();
        if !self.event_names.insert(event_name.clone()) {
            self.error_at_previous(&format!("Duplicate event name '{}'", event_name));
        }

        self.consume(TokenType::Equal, "Expected '=' after event name");

        if self.match_token(TokenType::Date) {
            self.consume(TokenType::LeftParen, "Expected '(' after 'date'");
            self.consume(
                TokenType::StringLiteral,
                "Expected date string inside date(...)",
            );
            let raw = &self.previous.lexeme;
            let date_string = raw[1..raw.len() - 1].to_string();
            self.consume(TokenType::RightParen, "Expected ')' after date string");

            self.emit_constant(Value::Identifier(event_name));
            self.emit_constant(Value::Str(date_string));
            self.emit_byte(u8::from(OpCode::DefineEvent));
            return;
        }

        if self.match_token(TokenType::TriggeredBy) {
            self.consume(TokenType::LeftParen, "Expected '(' after 'triggered_by'");
            self.consume(
                TokenType::Identifier,
                "Expected party identifier inside triggered_by(...)",
            );
            let party_role = self.previous.lexeme.clone();
            self.consume(
                TokenType::RightParen,
                "Expected ')' after triggered_by argument",
            );

            self.emit_constant(Value::Identifier(event_name));
            self.emit_constant(Value::Identifier(party_role));
            self.emit_byte(u8::from(OpCode::DefineEvent));
            return;
        }

        self.error_at_current("Expected event expression: date(\"...\") or triggered_by(identifier)");
        if !self.check(TokenType::Eof) {
            self.advance();
        }
    }

    /// Parse:
    ///   term NAME = NUMBER (calendar_days|business_days) from IDENTIFIER
    ///   term NAME = NUMBER (calendar_days|business_days) from breach_of(IDENTIFIER)
    ///
    /// Emits:
    ///   Constant(term_name), Constant(amount), Constant(unit), Constant(anchor), DefineTerm
    fn term_decl(&mut self) {
        self.consume(TokenType::Term, "Expected 'term'");
        self.consume(TokenType::Identifier, "Expected term name");
        let term_name = self.previous.lexeme.clone();
        if !self.term_names.insert(term_name.clone()) {
            self.error_at_previous(&format!("Duplicate term name '{}'", term_name));
        }

        self.consume(TokenType::Equal, "Expected '=' after term name");

        self.consume(TokenType::Number, "Expected duration number");
        let amount_lexeme = self.previous.lexeme.clone();
        let amount = match amount_lexeme.parse::<f64>() {
            Ok(n) => n,
            Err(_) => {
                self.error_at_previous("Invalid duration number");
                return;
            }
        };

        let unit = if self.match_token(TokenType::CalendarDays) {
            "calendar_days".to_string()
        } else if self.match_token(TokenType::BusinessDays) {
            "business_days".to_string()
        } else {
            self.error_at_current("Expected duration unit: 'calendar_days' or 'business_days'");
            if !self.check(TokenType::Eof) {
                self.advance();
            }
            return;
        };

        self.consume(TokenType::From, "Expected 'from' after term duration");

        let anchor = if self.match_token(TokenType::Identifier) {
            self.previous.lexeme.clone()
        } else if self.match_token(TokenType::BreachOf) {
            self.consume(TokenType::LeftParen, "Expected '(' after 'breach_of'");
            self.consume(
                TokenType::Identifier,
                "Expected identifier inside breach_of(...)",
            );
            let breached_name = self.previous.lexeme.clone();
            self.consume(TokenType::RightParen, "Expected ')' after breach_of argument");
            format!("breach_of({})", breached_name)
        } else {
            self.error_at_current("Expected term anchor identifier or breach_of(identifier)");
            if !self.check(TokenType::Eof) {
                self.advance();
            }
            return;
        };

        self.emit_constant(Value::Identifier(term_name));
        self.emit_constant(Value::Num(amount));
        self.emit_constant(Value::Str(unit));
        self.emit_constant(Value::Identifier(anchor));
        self.emit_byte(u8::from(OpCode::DefineTerm));
    }

    /// Parse:
    ///   obligation NAME {
    ///     party: IDENTIFIER
    ///     action: STRING
    ///     due: IDENTIFIER | NUMBER (calendar_days|business_days) from IDENTIFIER
    ///     condition: condition_expr
    ///   }
    ///
    /// Emits:
    ///   Constant(name), BeginObligation, [field emits], EndObligation
    fn obligation_decl(&mut self) {
        self.consume(TokenType::Obligation, "Expected 'obligation'");
        self.consume(TokenType::Identifier, "Expected obligation name");
        let obligation_name = self.previous.lexeme.clone();
        if !self.obligation_names.insert(obligation_name.clone()) {
            self.error_at_previous(&format!("Duplicate obligation name '{}'", obligation_name));
        }

        self.consume(TokenType::LeftBrace, "Expected '{' after obligation name");

        self.emit_constant(Value::Identifier(obligation_name));
        self.emit_byte(u8::from(OpCode::BeginObligation));

        self.parse_obligation_fields_until_right_brace();

        self.consume(TokenType::RightBrace, "Expected '}' after obligation block");
        self.emit_byte(u8::from(OpCode::EndObligation));
    }

    /// Parse:
    ///   remedy NAME on breach_of(TARGET) { ... }
    ///
    /// Body forms:
    ///   - direct obligation-like fields
    ///   - one or more `phase` declarations
    fn remedy_decl(&mut self) {
        self.consume(TokenType::Remedy, "Expected 'remedy'");
        self.consume(TokenType::Identifier, "Expected remedy name");
        let remedy_name = self.previous.lexeme.clone();
        if !self.remedy_names.insert(remedy_name.clone()) {
            self.error_at_previous(&format!("Duplicate remedy name '{}'", remedy_name));
        }

        self.consume(TokenType::On, "Expected 'on' after remedy name");
        self.consume(TokenType::BreachOf, "Expected 'breach_of' after 'on'");
        self.consume(TokenType::LeftParen, "Expected '(' after 'breach_of'");
        self.consume(
            TokenType::Identifier,
            "Expected breach target identifier in breach_of(...)",
        );
        let breach_target = self.previous.lexeme.clone();
        self.consume(TokenType::RightParen, "Expected ')' after breach_of target");
        self.consume(TokenType::LeftBrace, "Expected '{' after remedy header");

        self.emit_constant(Value::Identifier(remedy_name));
        self.emit_constant(Value::Identifier(breach_target));
        self.emit_byte(u8::from(OpCode::BeginRemedy));

        if self.check(TokenType::Phase) {
            let mut phase_names = HashSet::new();
            while self.check(TokenType::Phase) && !self.check(TokenType::Eof) {
                self.phase_decl(&mut phase_names);
            }
            while !self.check(TokenType::RightBrace) && !self.check(TokenType::Eof) {
                self.error_at_current("Expected 'phase' or '}' in remedy body");
                self.advance();
            }
        } else {
            self.parse_obligation_fields_until_right_brace();
        }

        self.consume(TokenType::RightBrace, "Expected '}' after remedy block");
        self.emit_byte(u8::from(OpCode::EndRemedy));
    }

    /// Parse:
    ///   phase NAME [on breach_of(TARGET)] { fields... }
    fn phase_decl(&mut self, phase_names: &mut HashSet<String>) {
        self.consume(TokenType::Phase, "Expected 'phase'");
        self.consume(TokenType::Identifier, "Expected phase name");
        let phase_name = self.previous.lexeme.clone();
        if !phase_names.insert(phase_name.clone()) {
            self.error_at_previous(&format!("Duplicate phase name '{}'", phase_name));
        }

        let phase_breach_target = if self.match_token(TokenType::On) {
            self.consume(TokenType::BreachOf, "Expected 'breach_of' after 'on'");
            self.consume(TokenType::LeftParen, "Expected '(' after 'breach_of'");
            self.consume(
                TokenType::Identifier,
                "Expected breach target identifier in breach_of(...)",
            );
            let target = self.previous.lexeme.clone();
            self.consume(TokenType::RightParen, "Expected ')' after breach_of target");
            Some(target)
        } else {
            None
        };

        self.consume(TokenType::LeftBrace, "Expected '{' after phase header");

        self.emit_constant(Value::Identifier(phase_name));
        if let Some(target) = phase_breach_target {
            self.emit_constant(Value::Identifier(format!("breach_of({})", target)));
        }
        self.emit_byte(u8::from(OpCode::BeginPhase));

        self.parse_obligation_fields_until_right_brace();
        self.consume(TokenType::RightBrace, "Expected '}' after phase block");
        self.emit_byte(u8::from(OpCode::EndPhase));
    }

    fn parse_obligation_fields_until_right_brace(&mut self) {
        while !self.check(TokenType::RightBrace) && !self.check(TokenType::Eof) {
            match self.current.token_type {
                TokenType::Party => self.obligation_party_field(),
                TokenType::Action => self.obligation_action_field(),
                TokenType::Due => self.obligation_due_field(),
                TokenType::Condition => self.obligation_condition_field(),
                _ => {
                    self.error_at_current(
                        "Expected obligation field (party, action, due, or condition)",
                    );
                    self.advance();
                }
            }
        }
    }

    fn obligation_party_field(&mut self) {
        self.consume(TokenType::Party, "Expected 'party'");
        self.consume(TokenType::Colon, "Expected ':' after 'party'");
        self.consume(TokenType::Identifier, "Expected party role identifier");
        let party_role = self.previous.lexeme.clone();

        self.emit_constant(Value::Identifier(party_role));
        self.emit_byte(u8::from(OpCode::SetParty));
    }

    fn obligation_action_field(&mut self) {
        self.consume(TokenType::Action, "Expected 'action'");
        self.consume(TokenType::Colon, "Expected ':' after 'action'");
        self.consume(TokenType::StringLiteral, "Expected action string");
        let action = Self::strip_quotes(&self.previous.lexeme);

        self.emit_constant(Value::Str(action));
        self.emit_byte(u8::from(OpCode::SetAction));
    }

    fn obligation_due_field(&mut self) {
        self.consume(TokenType::Due, "Expected 'due'");
        self.consume(TokenType::Colon, "Expected ':' after 'due'");

        if self.match_token(TokenType::Identifier) {
            let term_ref = self.previous.lexeme.clone();
            self.emit_constant(Value::Identifier(term_ref));
            self.emit_byte(u8::from(OpCode::SetDue));
            return;
        }

        if self.match_token(TokenType::Number) {
            let amount = self.previous.lexeme.clone();
            let unit = if self.match_token(TokenType::CalendarDays) {
                "calendar_days"
            } else if self.match_token(TokenType::BusinessDays) {
                "business_days"
            } else {
                self.error_at_current("Expected duration unit: 'calendar_days' or 'business_days'");
                return;
            };

            self.consume(TokenType::From, "Expected 'from' in inline due expression");
            self.consume(
                TokenType::Identifier,
                "Expected anchor identifier after 'from' in due expression",
            );
            let anchor = self.previous.lexeme.clone();

            let raw_due = format!("{} {} from {}", amount, unit, anchor);
            self.emit_constant(Value::Str(raw_due));
            self.emit_byte(u8::from(OpCode::SetDue));
            return;
        }

        self.error_at_current(
            "Expected due value: term identifier or inline 'NUMBER unit from IDENTIFIER'",
        );
    }

    fn obligation_condition_field(&mut self) {
        self.consume(TokenType::Condition, "Expected 'condition'");
        self.consume(TokenType::Colon, "Expected ':' after 'condition'");

        if self.condition_expr() {
            self.emit_byte(u8::from(OpCode::SetCondition));
        }
    }

    /// Parse condition expressions with precedence:
    ///   - atoms: after(...), before(...), satisfied(...), occurred(...), (...)
    ///   - `and` binds tighter than `or`
    ///
    /// Emits each atom as:
    ///   Constant(arg), Condition*
    /// and each chain link as:
    ///   ConditionAnd / ConditionOr
    fn condition_expr(&mut self) -> bool {
        self.condition_or_expr()
    }

    fn condition_or_expr(&mut self) -> bool {
        if !self.condition_and_expr() {
            return false;
        }

        while self.match_token(TokenType::Or) {
            if !self.condition_and_expr() {
                self.error_at_current("Expected condition expression after 'or'");
                return false;
            }
            self.emit_byte(u8::from(OpCode::ConditionOr));
        }

        true
    }

    fn condition_and_expr(&mut self) -> bool {
        if !self.condition_primary() {
            return false;
        }

        while self.match_token(TokenType::And) {
            if !self.condition_primary() {
                self.error_at_current("Expected condition expression after 'and'");
                return false;
            }
            self.emit_byte(u8::from(OpCode::ConditionAnd));
        }

        true
    }

    fn condition_primary(&mut self) -> bool {
        if self.match_token(TokenType::LeftParen) {
            if !self.condition_or_expr() {
                return false;
            }
            self.consume(
                TokenType::RightParen,
                "Expected ')' after grouped condition expression",
            );
            return true;
        }
        self.condition_atom()
    }

    fn condition_atom(&mut self) -> bool {
        if self.match_token(TokenType::After) {
            self.consume(TokenType::LeftParen, "Expected '(' after 'after'");
            self.consume(
                TokenType::Identifier,
                "Expected identifier inside after(...)",
            );
            let arg = self.previous.lexeme.clone();
            self.consume(TokenType::RightParen, "Expected ')' after after(...) argument");
            self.emit_constant(Value::Identifier(arg));
            self.emit_byte(u8::from(OpCode::ConditionAfter));
            return true;
        }

        if self.match_token(TokenType::Before) {
            self.consume(TokenType::LeftParen, "Expected '(' after 'before'");
            self.consume(
                TokenType::Identifier,
                "Expected identifier inside before(...)",
            );
            let arg = self.previous.lexeme.clone();
            self.consume(TokenType::RightParen, "Expected ')' after before(...) argument");
            self.emit_constant(Value::Identifier(arg));
            self.emit_byte(u8::from(OpCode::ConditionBefore));
            return true;
        }

        if self.match_token(TokenType::Satisfied) {
            self.consume(TokenType::LeftParen, "Expected '(' after 'satisfied'");
            self.consume(
                TokenType::Identifier,
                "Expected identifier inside satisfied(...)",
            );
            let arg = self.previous.lexeme.clone();
            self.consume(
                TokenType::RightParen,
                "Expected ')' after satisfied(...) argument",
            );
            self.emit_constant(Value::Identifier(arg));
            self.emit_byte(u8::from(OpCode::ConditionSatisfied));
            return true;
        }

        if self.match_token(TokenType::Occurred) {
            self.consume(TokenType::LeftParen, "Expected '(' after 'occurred'");
            self.consume(
                TokenType::Identifier,
                "Expected identifier inside occurred(...)",
            );
            let arg = self.previous.lexeme.clone();
            self.consume(
                TokenType::RightParen,
                "Expected ')' after occurred(...) argument",
            );
            self.emit_constant(Value::Identifier(arg));
            self.emit_byte(u8::from(OpCode::ConditionOccurred));
            return true;
        }

        self.error_at_current(
            "Expected condition expression: after(...), before(...), satisfied(...), occurred(...), or grouped (...)",
        );
        false
    }

    fn strip_quotes(raw: &str) -> String {
        raw[1..raw.len() - 1].to_string()
    }

    // ── bytecode emission ───────────────────────────────────────────

    /// Emit a single byte into the chunk, tagged with the line number
    /// of the most recently consumed token (`previous`).
    fn emit_byte(&mut self, byte: u8) {
        let line = self.previous.span.line;
        self.chunk.write(byte, line);
    }

    /// Emit two consecutive bytes.
    fn emit_bytes(&mut self, a: u8, b: u8) {
        self.emit_byte(a);
        self.emit_byte(b);
    }

    /// Push `value` into the constant pool and emit a `Constant` instruction
    /// that references it.
    fn emit_constant(&mut self, value: Value) {
        let idx = self.chunk.add_constant(value);
        self.emit_bytes(u8::from(OpCode::Constant), idx);
    }

    /// Emit the final `Return` opcode.
    fn emit_return(&mut self) {
        self.emit_byte(u8::from(OpCode::Return));
    }

    // ── error handling ──────────────────────────────────────────────

    /// Report an error located at the *current* (not-yet-consumed) token.
    fn error_at_current(&mut self, msg: &str) {
        let token = self.current.clone();
        self.error_at(&token, msg);
    }

    /// Report an error located at the *previous* (just-consumed) token.
    #[allow(dead_code)]
    fn error_at_previous(&mut self, msg: &str) {
        let token = self.previous.clone();
        self.error_at(&token, msg);
    }

    /// Core error reporter.
    ///
    /// If already in panic mode, suppress cascading errors.
    /// Otherwise, record the error, flip `had_error` and `panic_mode`,
    /// and print the error to stderr.
    fn error_at(&mut self, token: &Token, msg: &str) {
        if self.panic_mode {
            return; // suppress cascading errors
        }
        self.panic_mode = true;
        self.had_error = true;

        let error = KontraError::compile(token.span.clone(), msg.to_string());
        eprintln!("{}", error);
        self.errors.push(error);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::OpCode;

    #[test]
    fn compile_empty_string_produces_return() {
        let chunk = Compiler::compile("").expect("should compile empty source");

        // The only opcode should be Return.
        assert_eq!(chunk.code.len(), 1);
        assert_eq!(OpCode::from(chunk.code[0]), OpCode::Return);
    }

    #[test]
    fn compile_whitespace_only_produces_return() {
        let chunk = Compiler::compile("   \n\n\t  ")
            .expect("should compile whitespace-only source");

        assert_eq!(chunk.code.len(), 1);
        assert_eq!(OpCode::from(chunk.code[0]), OpCode::Return);
    }

    #[test]
    fn compile_tracks_error_on_bad_token() {
        // An unterminated string should cause a compile error.
        let result = Compiler::compile(r#""oops"#);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("Unterminated string"),
            "expected 'Unterminated string' in error, got: {}",
            msg
        );
    }

    // ── Block 7: Contract & Parties ─────────────────────────────────

    #[test]
    fn compile_single_party_contract() {
        let source = r#"contract Foo { parties { buyer: "Alice" } }"#;
        let chunk = Compiler::compile(source).expect("should compile");

        // Constants: Identifier("buyer"), Str("Alice")
        assert_eq!(chunk.constants.len(), 2);
        assert_eq!(chunk.constants[0], Value::Identifier("buyer".into()));
        assert_eq!(chunk.constants[1], Value::Str("Alice".into()));

        // Byte sequence: [Constant, 0, Constant, 1, DefineParty, Return]
        assert_eq!(chunk.code.len(), 6);
        assert_eq!(OpCode::from(chunk.code[0]), OpCode::Constant);
        assert_eq!(chunk.code[1], 0); // index of "buyer"
        assert_eq!(OpCode::from(chunk.code[2]), OpCode::Constant);
        assert_eq!(chunk.code[3], 1); // index of "Alice"
        assert_eq!(OpCode::from(chunk.code[4]), OpCode::DefineParty);
        assert_eq!(OpCode::from(chunk.code[5]), OpCode::Return);

        // Verify disassembly contains DefineParty
        let dis = chunk.disassemble_to_string("single-party");
        assert!(dis.contains("DefineParty"), "disassembly:\n{}", dis);
    }

    #[test]
    fn compile_multi_party_contract() {
        let source = r#"contract Deal {
            parties {
                buyer: "Alice"
                seller: "Bob"
            }
        }"#;
        let chunk = Compiler::compile(source).expect("should compile");

        // Four constants: buyer, Alice, seller, Bob
        assert_eq!(chunk.constants.len(), 4);
        assert_eq!(chunk.constants[0], Value::Identifier("buyer".into()));
        assert_eq!(chunk.constants[1], Value::Str("Alice".into()));
        assert_eq!(chunk.constants[2], Value::Identifier("seller".into()));
        assert_eq!(chunk.constants[3], Value::Str("Bob".into()));

        // Walk the bytecode and count DefineParty opcodes (skipping operands).
        let mut i = 0;
        let mut define_count = 0;
        while i < chunk.code.len() {
            let op = OpCode::from(chunk.code[i]);
            if op == OpCode::Constant {
                i += 2; // opcode + index operand
            } else {
                if op == OpCode::DefineParty {
                    define_count += 1;
                }
                i += 1;
            }
        }
        assert_eq!(define_count, 2, "expected 2 DefineParty opcodes");

        // Last byte is Return
        assert_eq!(
            OpCode::from(*chunk.code.last().unwrap()),
            OpCode::Return,
        );
    }

    #[test]
    fn compile_contract_missing_closing_brace() {
        let source = r#"contract Foo { parties { buyer: "Alice" }"#;
        let result = Compiler::compile(source);
        assert!(result.is_err());

        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Expected '}'"),
            "expected closing-brace error, got: {}",
            msg
        );
    }

    #[test]
    fn compile_contract_missing_contract_keyword() {
        let source = r#"parties { buyer: "Alice" }"#;
        let result = Compiler::compile(source);
        assert!(result.is_err());

        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Expected 'contract'"),
            "expected keyword error, got: {}",
            msg
        );
    }

    #[test]
    fn compile_empty_contract_body_produces_return_only() {
        let source = r#"contract Empty {}"#;
        let chunk = Compiler::compile(source).expect("empty contract should compile");
        assert_eq!(chunk.code.len(), 1);
        assert_eq!(OpCode::from(chunk.code[0]), OpCode::Return);
    }

    // ── Block 8: Events ─────────────────────────────────────────────

    #[test]
    fn compile_event_date_decl() {
        let source = r#"contract Foo { event Effective = date("2026-03-01") }"#;
        let chunk = Compiler::compile(source).expect("should compile");

        assert_eq!(chunk.constants.len(), 2);
        assert_eq!(chunk.constants[0], Value::Identifier("Effective".into()));
        assert_eq!(chunk.constants[1], Value::Str("2026-03-01".into()));

        // [Constant, 0, Constant, 1, DefineEvent, Return]
        assert_eq!(chunk.code.len(), 6);
        assert_eq!(OpCode::from(chunk.code[0]), OpCode::Constant);
        assert_eq!(chunk.code[1], 0);
        assert_eq!(OpCode::from(chunk.code[2]), OpCode::Constant);
        assert_eq!(chunk.code[3], 1);
        assert_eq!(OpCode::from(chunk.code[4]), OpCode::DefineEvent);
        assert_eq!(OpCode::from(chunk.code[5]), OpCode::Return);
    }

    #[test]
    fn compile_event_triggered_by_decl() {
        let source = r#"contract Foo { event Delivery = triggered_by(seller) }"#;
        let chunk = Compiler::compile(source).expect("should compile");

        assert_eq!(chunk.constants.len(), 2);
        assert_eq!(chunk.constants[0], Value::Identifier("Delivery".into()));
        assert_eq!(chunk.constants[1], Value::Identifier("seller".into()));

        // [Constant, 0, Constant, 1, DefineEvent, Return]
        assert_eq!(chunk.code.len(), 6);
        assert_eq!(OpCode::from(chunk.code[0]), OpCode::Constant);
        assert_eq!(chunk.code[1], 0);
        assert_eq!(OpCode::from(chunk.code[2]), OpCode::Constant);
        assert_eq!(chunk.code[3], 1);
        assert_eq!(OpCode::from(chunk.code[4]), OpCode::DefineEvent);
        assert_eq!(OpCode::from(chunk.code[5]), OpCode::Return);
    }

    // ── Block 9: Terms ──────────────────────────────────────────────

    #[test]
    fn compile_term_calendar_days_from_event() {
        let source = r#"contract Foo { term CurePeriod = 10 calendar_days from Effective }"#;
        let chunk = Compiler::compile(source).expect("should compile");

        assert_eq!(chunk.constants.len(), 4);
        assert_eq!(chunk.constants[0], Value::Identifier("CurePeriod".into()));
        assert_eq!(chunk.constants[1], Value::Num(10.0));
        assert_eq!(chunk.constants[2], Value::Str("calendar_days".into()));
        assert_eq!(chunk.constants[3], Value::Identifier("Effective".into()));

        // [Constant,0, Constant,1, Constant,2, Constant,3, DefineTerm, Return]
        assert_eq!(chunk.code.len(), 10);
        assert_eq!(OpCode::from(chunk.code[0]), OpCode::Constant);
        assert_eq!(chunk.code[1], 0);
        assert_eq!(OpCode::from(chunk.code[2]), OpCode::Constant);
        assert_eq!(chunk.code[3], 1);
        assert_eq!(OpCode::from(chunk.code[4]), OpCode::Constant);
        assert_eq!(chunk.code[5], 2);
        assert_eq!(OpCode::from(chunk.code[6]), OpCode::Constant);
        assert_eq!(chunk.code[7], 3);
        assert_eq!(OpCode::from(chunk.code[8]), OpCode::DefineTerm);
        assert_eq!(OpCode::from(chunk.code[9]), OpCode::Return);
    }

    #[test]
    fn compile_term_business_days_from_breach_of() {
        let source =
            r#"contract Foo { term CurePeriod = 10 business_days from breach_of(DeliverSoftware) }"#;
        let chunk = Compiler::compile(source).expect("should compile");

        assert_eq!(chunk.constants.len(), 4);
        assert_eq!(chunk.constants[0], Value::Identifier("CurePeriod".into()));
        assert_eq!(chunk.constants[1], Value::Num(10.0));
        assert_eq!(chunk.constants[2], Value::Str("business_days".into()));
        assert_eq!(
            chunk.constants[3],
            Value::Identifier("breach_of(DeliverSoftware)".into())
        );

        assert_eq!(chunk.code.len(), 10);
        assert_eq!(OpCode::from(chunk.code[8]), OpCode::DefineTerm);
        assert_eq!(OpCode::from(chunk.code[9]), OpCode::Return);
    }

    #[test]
    fn compile_term_missing_from_reports_error() {
        let source = r#"contract Foo { term CurePeriod = 10 calendar_days Effective }"#;
        let result = Compiler::compile(source);
        assert!(result.is_err());

        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Expected 'from' after term duration"),
            "expected missing-from error, got: {}",
            msg
        );
    }

    // ── Block 10: Obligations ───────────────────────────────────────

    #[test]
    fn compile_obligation_all_fields_with_due_identifier() {
        let source = r#"contract Foo {
            obligation PayLicenseFee {
                party: licensee
                action: "Pay $50,000 license fee"
                due: AcceptancePeriod
                condition: satisfied(DeliverSoftware) and occurred(AcceptanceNotice)
            }
        }"#;
        let chunk = Compiler::compile(source).expect("should compile");

        assert_eq!(chunk.constants.len(), 6);
        assert_eq!(chunk.constants[0], Value::Identifier("PayLicenseFee".into()));
        assert_eq!(chunk.constants[1], Value::Identifier("licensee".into()));
        assert_eq!(
            chunk.constants[2],
            Value::Str("Pay $50,000 license fee".into())
        );
        assert_eq!(chunk.constants[3], Value::Identifier("AcceptancePeriod".into()));
        assert_eq!(chunk.constants[4], Value::Identifier("DeliverSoftware".into()));
        assert_eq!(chunk.constants[5], Value::Identifier("AcceptanceNotice".into()));

        // [Constant,0, BeginObligation, Constant,1, SetParty, Constant,2, SetAction,
        //  Constant,3, SetDue, Constant,4, ConditionSatisfied, Constant,5,
        //  ConditionOccurred, ConditionAnd, SetCondition, EndObligation, Return]
        assert_eq!(chunk.code.len(), 22);
        assert_eq!(OpCode::from(chunk.code[0]), OpCode::Constant);
        assert_eq!(chunk.code[1], 0);
        assert_eq!(OpCode::from(chunk.code[2]), OpCode::BeginObligation);
        assert_eq!(OpCode::from(chunk.code[3]), OpCode::Constant);
        assert_eq!(chunk.code[4], 1);
        assert_eq!(OpCode::from(chunk.code[5]), OpCode::SetParty);
        assert_eq!(OpCode::from(chunk.code[6]), OpCode::Constant);
        assert_eq!(chunk.code[7], 2);
        assert_eq!(OpCode::from(chunk.code[8]), OpCode::SetAction);
        assert_eq!(OpCode::from(chunk.code[9]), OpCode::Constant);
        assert_eq!(chunk.code[10], 3);
        assert_eq!(OpCode::from(chunk.code[11]), OpCode::SetDue);
        assert_eq!(OpCode::from(chunk.code[12]), OpCode::Constant);
        assert_eq!(chunk.code[13], 4);
        assert_eq!(OpCode::from(chunk.code[14]), OpCode::ConditionSatisfied);
        assert_eq!(OpCode::from(chunk.code[15]), OpCode::Constant);
        assert_eq!(chunk.code[16], 5);
        assert_eq!(OpCode::from(chunk.code[17]), OpCode::ConditionOccurred);
        assert_eq!(OpCode::from(chunk.code[18]), OpCode::ConditionAnd);
        assert_eq!(OpCode::from(chunk.code[19]), OpCode::SetCondition);
        assert_eq!(OpCode::from(chunk.code[20]), OpCode::EndObligation);
        assert_eq!(OpCode::from(chunk.code[21]), OpCode::Return);
    }

    #[test]
    fn compile_obligation_inline_due_emits_set_due_with_raw_constant() {
        let source = r#"contract Foo {
            obligation PayLicenseFee {
                party: licensee
                action: "Pay fee"
                due: 15 calendar_days from AcceptanceNotice
                condition: occurred(AcceptanceNotice)
            }
        }"#;
        let chunk = Compiler::compile(source).expect("should compile");

        assert_eq!(
            chunk.constants[3],
            Value::Str("15 calendar_days from AcceptanceNotice".into())
        );
        assert_eq!(chunk.constants[4], Value::Identifier("AcceptanceNotice".into()));

        assert_eq!(chunk.code.len(), 18);
        assert_eq!(OpCode::from(chunk.code[11]), OpCode::SetDue);
        assert_eq!(OpCode::from(chunk.code[14]), OpCode::ConditionOccurred);
        assert_eq!(OpCode::from(chunk.code[15]), OpCode::SetCondition);
        assert_eq!(OpCode::from(chunk.code[16]), OpCode::EndObligation);
        assert_eq!(OpCode::from(chunk.code[17]), OpCode::Return);
    }

    #[test]
    fn compile_obligation_unknown_field_reports_error() {
        let source = r#"contract Foo {
            obligation Bad {
                foo: bar
            }
        }"#;
        let result = Compiler::compile(source);
        assert!(result.is_err());

        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Expected obligation field"),
            "expected obligation field error, got: {}",
            msg
        );
    }

    #[test]
    fn compile_duplicate_obligation_name_reports_error() {
        let source = r#"contract Foo {
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software"
                due: 10 calendar_days from Effective
            }
            obligation DeliverSoftware {
                party: seller
                action: "Deliver software again"
                due: 20 calendar_days from Effective
            }
        }"#;

        let result = Compiler::compile(source);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Duplicate obligation name 'DeliverSoftware'"),
            "expected duplicate-obligation error, got: {}",
            msg
        );
    }

    // ── Block 11: Remedies & Phases ─────────────────────────────────

    #[test]
    fn compile_remedy_with_direct_fields() {
        let source = r#"contract Foo {
            remedy LateFee on breach_of(PayLicenseFee) {
                party: licensee
                action: "Pay interest"
                due: CurePeriod
                condition: occurred(DefaultNotice)
            }
        }"#;
        let chunk = Compiler::compile(source).expect("should compile");

        assert_eq!(chunk.constants[0], Value::Identifier("LateFee".into()));
        assert_eq!(chunk.constants[1], Value::Identifier("PayLicenseFee".into()));
        assert_eq!(chunk.constants[2], Value::Identifier("licensee".into()));
        assert_eq!(chunk.constants[3], Value::Str("Pay interest".into()));
        assert_eq!(chunk.constants[4], Value::Identifier("CurePeriod".into()));
        assert_eq!(chunk.constants[5], Value::Identifier("DefaultNotice".into()));

        assert_eq!(chunk.code.len(), 20);
        assert_eq!(OpCode::from(chunk.code[4]), OpCode::BeginRemedy);
        assert_eq!(OpCode::from(chunk.code[7]), OpCode::SetParty);
        assert_eq!(OpCode::from(chunk.code[10]), OpCode::SetAction);
        assert_eq!(OpCode::from(chunk.code[13]), OpCode::SetDue);
        assert_eq!(OpCode::from(chunk.code[16]), OpCode::ConditionOccurred);
        assert_eq!(OpCode::from(chunk.code[17]), OpCode::SetCondition);
        assert_eq!(OpCode::from(chunk.code[18]), OpCode::EndRemedy);
        assert_eq!(OpCode::from(chunk.code[19]), OpCode::Return);
    }

    #[test]
    fn compile_remedy_with_phases_emits_phase_opcodes() {
        let source = r#"contract Foo {
            remedy CureOrTerminate on breach_of(DeliverSoftware) {
                phase Cure {
                    party: licensor
                    action: "Deliver software in cure period"
                    due: CurePeriod
                    condition: after(Effective)
                }
                phase Terminate on breach_of(Cure) {
                    action: "Terminate contract"
                    condition: satisfied(Cure) and occurred(TerminationNotice)
                }
            }
        }"#;
        let chunk = Compiler::compile(source).expect("should compile");

        assert!(
            chunk
                .constants
                .contains(&Value::Identifier("breach_of(Cure)".into()))
        );

        let mut begin_phase = 0;
        let mut end_phase = 0;
        let mut begin_remedy = 0;
        let mut end_remedy = 0;
        let mut condition_and = 0;

        let mut i = 0;
        while i < chunk.code.len() {
            let op = OpCode::from(chunk.code[i]);
            match op {
                OpCode::Constant => {
                    i += 2;
                    continue;
                }
                OpCode::BeginPhase => begin_phase += 1,
                OpCode::EndPhase => end_phase += 1,
                OpCode::BeginRemedy => begin_remedy += 1,
                OpCode::EndRemedy => end_remedy += 1,
                OpCode::ConditionAnd => condition_and += 1,
                _ => {}
            }
            i += 1;
        }

        assert_eq!(begin_remedy, 1);
        assert_eq!(end_remedy, 1);
        assert_eq!(begin_phase, 2);
        assert_eq!(end_phase, 2);
        assert_eq!(condition_and, 1);
        assert_eq!(
            OpCode::from(*chunk.code.last().expect("code should not be empty")),
            OpCode::Return
        );
    }

    #[test]
    fn compile_duplicate_party_role_reports_error() {
        let source = r#"contract Foo {
            parties {
                buyer: "Alice"
                buyer: "Alicia"
            }
        }"#;

        let result = Compiler::compile(source);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Duplicate party role 'buyer'"),
            "expected duplicate-party error, got: {}",
            msg
        );
    }

    #[test]
    fn compile_duplicate_event_name_reports_error() {
        let source = r#"contract Foo {
            event Effective = date("2026-03-01")
            event Effective = date("2026-03-02")
        }"#;

        let result = Compiler::compile(source);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Duplicate event name 'Effective'"),
            "expected duplicate-event error, got: {}",
            msg
        );
    }

    #[test]
    fn compile_duplicate_term_name_reports_error() {
        let source = r#"contract Foo {
            term DeliveryPeriod = 10 calendar_days from Effective
            term DeliveryPeriod = 20 calendar_days from Effective
        }"#;

        let result = Compiler::compile(source);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Duplicate term name 'DeliveryPeriod'"),
            "expected duplicate-term error, got: {}",
            msg
        );
    }

    #[test]
    fn compile_duplicate_remedy_name_reports_error() {
        let source = r#"contract Foo {
            remedy LateFee on breach_of(PayLicenseFee) {
                action: "Pay interest"
            }
            remedy LateFee on breach_of(PayLicenseFee) {
                action: "Pay more interest"
            }
        }"#;

        let result = Compiler::compile(source);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Duplicate remedy name 'LateFee'"),
            "expected duplicate-remedy error, got: {}",
            msg
        );
    }

    #[test]
    fn compile_duplicate_phase_name_reports_error() {
        let source = r#"contract Foo {
            remedy CureOrTerminate on breach_of(DeliverSoftware) {
                phase Cure {
                    action: "first"
                }
                phase Cure {
                    action: "second"
                }
            }
        }"#;

        let result = Compiler::compile(source);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("Duplicate phase name 'Cure'"),
            "expected duplicate-phase error, got: {}",
            msg
        );
    }

    #[test]
    fn compile_condition_mixed_and_or_emits_precedence_order() {
        let source = r#"contract Foo {
            obligation Pay {
                party: buyer
                action: "Pay fee"
                due: PaymentWindow
                condition: satisfied(DeliverSoftware) or occurred(AcceptanceNotice) and before(Cutoff)
            }
        }"#;

        let chunk = Compiler::compile(source).expect("should compile");
        let mut condition_ops = Vec::new();
        let mut i = 0;
        while i < chunk.code.len() {
            let op = OpCode::from(chunk.code[i]);
            match op {
                OpCode::Constant => {
                    i += 2;
                    continue;
                }
                OpCode::ConditionSatisfied
                | OpCode::ConditionOccurred
                | OpCode::ConditionBefore
                | OpCode::ConditionAnd
                | OpCode::ConditionOr
                | OpCode::SetCondition => condition_ops.push(op),
                _ => {}
            }
            i += 1;
        }

        assert_eq!(
            condition_ops,
            vec![
                OpCode::ConditionSatisfied,
                OpCode::ConditionOccurred,
                OpCode::ConditionBefore,
                OpCode::ConditionAnd,
                OpCode::ConditionOr,
                OpCode::SetCondition,
            ],
            "and should bind tighter than or",
        );
    }

    #[test]
    fn compile_condition_grouping_changes_opcode_order() {
        let source = r#"contract Foo {
            obligation Pay {
                party: buyer
                action: "Pay fee"
                due: PaymentWindow
                condition: (satisfied(DeliverSoftware) or occurred(AcceptanceNotice)) and before(Cutoff)
            }
        }"#;

        let chunk = Compiler::compile(source).expect("should compile");
        let mut condition_ops = Vec::new();
        let mut i = 0;
        while i < chunk.code.len() {
            let op = OpCode::from(chunk.code[i]);
            match op {
                OpCode::Constant => {
                    i += 2;
                    continue;
                }
                OpCode::ConditionSatisfied
                | OpCode::ConditionOccurred
                | OpCode::ConditionBefore
                | OpCode::ConditionAnd
                | OpCode::ConditionOr
                | OpCode::SetCondition => condition_ops.push(op),
                _ => {}
            }
            i += 1;
        }

        assert_eq!(
            condition_ops,
            vec![
                OpCode::ConditionSatisfied,
                OpCode::ConditionOccurred,
                OpCode::ConditionOr,
                OpCode::ConditionBefore,
                OpCode::ConditionAnd,
                OpCode::SetCondition,
            ],
            "grouping should force or before and",
        );
    }
}
