use crate::error::Span;
use crate::token::{Token, TokenType};

pub struct Scanner {
    source: Vec<char>,
    start: usize,   // byte offset of current token start
    current: usize, // byte offset of current position
    line: usize,
    col: usize,
}

impl Scanner {
    pub fn new(source: &str) -> Self {
        Self {
            source: source.chars().collect(),
            start: 0,
            current: 0,
            line: 1,
            col: 1,
        }
    }

    /// Returns the next token from the source.
    pub fn scan_token(&mut self) -> Token {
        self.skip_whitespace();
        self.start = self.current;

        if self.is_at_end() {
            return self.make_token(TokenType::Eof);
        }

        let c = self.advance();

        match c {
            '{' => self.make_token(TokenType::LeftBrace),
            '}' => self.make_token(TokenType::RightBrace),
            '(' => self.make_token(TokenType::LeftParen),
            ')' => self.make_token(TokenType::RightParen),
            ':' => self.make_token(TokenType::Colon),
            '=' => self.make_token(TokenType::Equal),
            ',' => self.make_token(TokenType::Comma),
            '.' => self.make_token(TokenType::Dot),

            '"' => self.string(),

            c if c.is_ascii_digit() => self.number(),

            c if c.is_ascii_alphabetic() || c == '_' => self.identifier(),

            _ => self.error_token(&format!("Unexpected character '{}'", c)),
        }
    }

    // -- helpers --

    fn is_at_end(&self) -> bool {
        self.current >= self.source.len()
    }

    fn peek(&self) -> char {
        if self.is_at_end() {
            '\0'
        } else {
            self.source[self.current]
        }
    }

    fn advance(&mut self) -> char {
        let c = self.source[self.current];
        self.current += 1;
        self.col += 1;
        c
    }

    fn skip_whitespace(&mut self) {
        loop {
            if self.is_at_end() {
                return;
            }
            match self.peek() {
                ' ' | '\r' | '\t' => {
                    self.advance();
                }
                '\n' => {
                    self.line += 1;
                    self.col = 0; // advance() will bump to 1
                    self.advance();
                }
                '-' if self.current + 1 < self.source.len()
                    && self.source[self.current + 1] == '-' =>
                {
                    // line comment: consume until end of line
                    while !self.is_at_end() && self.peek() != '\n' {
                        self.advance();
                    }
                }
                _ => return,
            }
        }
    }

    fn string(&mut self) -> Token {
        while !self.is_at_end() && self.peek() != '"' {
            if self.peek() == '\n' {
                self.line += 1;
                self.col = 0;
            }
            self.advance();
        }

        if self.is_at_end() {
            return self.error_token("Unterminated string");
        }

        // consume the closing "
        self.advance();
        self.make_token(TokenType::StringLiteral)
    }

    fn number(&mut self) -> Token {
        while !self.is_at_end() && self.peek().is_ascii_digit() {
            self.advance();
        }
        self.make_token(TokenType::Number)
    }

    fn identifier(&mut self) -> Token {
        while !self.is_at_end() && (self.peek().is_ascii_alphanumeric() || self.peek() == '_') {
            self.advance();
        }
        let lexeme: String = self.source[self.start..self.current].iter().collect();
        let token_type = Self::keyword_type(&lexeme);
        self.make_token(token_type)
    }

    fn keyword_type(word: &str) -> TokenType {
        match word {
            "contract" => TokenType::Contract,
            "parties" => TokenType::Parties,
            "event" => TokenType::Event,
            "term" => TokenType::Term,
            "obligation" => TokenType::Obligation,
            "remedy" => TokenType::Remedy,
            "phase" => TokenType::Phase,
            "party" => TokenType::Party,
            "action" => TokenType::Action,
            "due" => TokenType::Due,
            "condition" => TokenType::Condition,
            "effect" => TokenType::Effect,
            "from" => TokenType::From,
            "on" => TokenType::On,
            "and" => TokenType::And,
            "or" => TokenType::Or,
            "after" => TokenType::After,
            "before" => TokenType::Before,
            "satisfied" => TokenType::Satisfied,
            "breach_of" => TokenType::BreachOf,
            "occurred" => TokenType::Occurred,
            "triggered_by" => TokenType::TriggeredBy,
            "date" => TokenType::Date,
            "calendar_days" => TokenType::CalendarDays,
            "business_days" => TokenType::BusinessDays,
            "terminate" => TokenType::Terminate,
            "recurring" => TokenType::Recurring,
            "until" => TokenType::Until,
            _ => TokenType::Identifier,
        }
    }

    fn make_token(&self, token_type: TokenType) -> Token {
        let lexeme: String = self.source[self.start..self.current].iter().collect();
        Token {
            token_type,
            lexeme,
            span: Span::new(
                self.line,
                self.col - (self.current - self.start),
                self.start,
                self.current,
            ),
        }
    }

    fn error_token(&self, message: &str) -> Token {
        Token {
            token_type: TokenType::Error,
            lexeme: message.to_string(),
            span: Span::new(
                self.line,
                self.col - (self.current - self.start),
                self.start,
                self.current,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: collect all tokens (including Eof) from a source string.
    fn scan_all(source: &str) -> Vec<Token> {
        let mut scanner = Scanner::new(source);
        let mut tokens = Vec::new();
        loop {
            let tok = scanner.scan_token();
            let is_eof = tok.token_type == TokenType::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        tokens
    }

    #[test]
    fn scan_small_contract_snippet() {
        let source = r#"contract Foo {
    parties {
        buyer: "Alice"
    }
    event Effective = date("2026-03-01")
    obligation Pay {
        party: buyer
        action: "Pay 100"
        due: 30 calendar_days from Effective
    }
}"#;
        let tokens = scan_all(source);
        let types: Vec<&TokenType> = tokens.iter().map(|t| &t.token_type).collect();

        assert_eq!(
            types,
            vec![
                // contract Foo {
                &TokenType::Contract,
                &TokenType::Identifier,
                &TokenType::LeftBrace,
                // parties {
                &TokenType::Parties,
                &TokenType::LeftBrace,
                // buyer: "Alice"
                &TokenType::Identifier,
                &TokenType::Colon,
                &TokenType::StringLiteral,
                // }
                &TokenType::RightBrace,
                // event Effective = date("2026-03-01")
                &TokenType::Event,
                &TokenType::Identifier,
                &TokenType::Equal,
                &TokenType::Date,
                &TokenType::LeftParen,
                &TokenType::StringLiteral,
                &TokenType::RightParen,
                // obligation Pay {
                &TokenType::Obligation,
                &TokenType::Identifier,
                &TokenType::LeftBrace,
                // party: buyer
                &TokenType::Party,
                &TokenType::Colon,
                &TokenType::Identifier,
                // action: "Pay 100"
                &TokenType::Action,
                &TokenType::Colon,
                &TokenType::StringLiteral,
                // due: 30 calendar_days from Effective
                &TokenType::Due,
                &TokenType::Colon,
                &TokenType::Number,
                &TokenType::CalendarDays,
                &TokenType::From,
                &TokenType::Identifier,
                // } }
                &TokenType::RightBrace,
                &TokenType::RightBrace,
                // Eof
                &TokenType::Eof,
            ]
        );

        // spot-check some lexemes
        assert_eq!(tokens[1].lexeme, "Foo");
        assert_eq!(tokens[7].lexeme, "\"Alice\"");
        assert_eq!(tokens[27].lexeme, "30");
    }

    #[test]
    fn line_comments_are_ignored() {
        let source = "contract Test {\n-- this is a comment\nparties {\n}\n}";
        let tokens = scan_all(source);
        let types: Vec<&TokenType> = tokens.iter().map(|t| &t.token_type).collect();

        assert_eq!(
            types,
            vec![
                &TokenType::Contract,
                &TokenType::Identifier,
                &TokenType::LeftBrace,
                &TokenType::Parties,
                &TokenType::LeftBrace,
                &TokenType::RightBrace,
                &TokenType::RightBrace,
                &TokenType::Eof,
            ]
        );
    }

    #[test]
    fn unterminated_string_produces_error_token() {
        let source = r#"contract "oops"#;
        let tokens = scan_all(source);

        let last_real = &tokens[tokens.len() - 2]; // token before Eof
        assert_eq!(last_real.token_type, TokenType::Error);
        assert_eq!(last_real.lexeme, "Unterminated string");
    }
}
