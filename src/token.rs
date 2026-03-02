use crate::error::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenType {
    // Keywords
    Contract,
    Parties,
    Event,
    Term,
    Obligation,
    Remedy,
    Phase,
    Party,
    Action,
    Due,
    Condition,
    Effect,
    From,
    On,
    And,
    Or,
    After,
    Before,
    Satisfied,
    BreachOf,
    Occurred,
    TriggeredBy,
    Date,
    CalendarDays,
    BusinessDays,
    Terminate,
    Recurring,
    Until,

    // Literals
    StringLiteral,
    Number,
    Identifier,

    // Symbols
    LeftBrace,
    RightBrace,
    LeftParen,
    RightParen,
    Colon,
    Equal,
    Comma,
    Dot,

    // Sentinels
    Eof,
    Error,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub token_type: TokenType,
    pub lexeme: String,
    pub span: Span,
}
