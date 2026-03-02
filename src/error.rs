use std::fmt;

/// Source location for error reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub col: usize,
    pub start: usize, // byte offset into source
    pub end: usize,   // byte offset into source
}

impl Span {
    pub fn new(line: usize, col: usize, start: usize, end: usize) -> Self {
        Self {
            line,
            col,
            start,
            end,
        }
    }
}

/// Unified error type for the Kontra language.
#[derive(Debug, Clone)]
pub enum KontraError {
    Scan {
        span: Span,
        message: String,
    },
    Compile {
        span: Span,
        message: String,
    },
    Runtime {
        line: Option<usize>,
        message: String,
    },
}

impl KontraError {
    pub fn scan(span: Span, msg: impl Into<String>) -> Self {
        Self::Scan {
            span,
            message: msg.into(),
        }
    }

    pub fn compile(span: Span, msg: impl Into<String>) -> Self {
        Self::Compile {
            span,
            message: msg.into(),
        }
    }

    pub fn runtime(line: Option<usize>, msg: impl Into<String>) -> Self {
        Self::Runtime {
            line,
            message: msg.into(),
        }
    }
}

impl fmt::Display for KontraError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Scan { span, message } => {
                write!(
                    f,
                    "[line {}, col {}] Scan error: {}",
                    span.line, span.col, message
                )
            }
            Self::Compile { span, message } => {
                write!(
                    f,
                    "[line {}, col {}] Compile error: {}",
                    span.line, span.col, message
                )
            }
            Self::Runtime {
                line: Some(l),
                message,
            } => {
                write!(f, "[line {}] Runtime error: {}", l, message)
            }
            Self::Runtime {
                line: None,
                message,
            } => {
                write!(f, "Runtime error: {}", message)
            }
        }
    }
}

impl std::error::Error for KontraError {}

/// Convenience result alias used throughout the crate.
pub type Result<T> = std::result::Result<T, KontraError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_fields() {
        let span = Span::new(5, 12, 40, 55);
        assert_eq!(span.line, 5);
        assert_eq!(span.col, 12);
        assert_eq!(span.start, 40);
        assert_eq!(span.end, 55);
    }

    #[test]
    fn scan_error_display() {
        let err = KontraError::scan(Span::new(3, 7, 20, 25), "unterminated string");
        let msg = err.to_string();
        assert_eq!(msg, "[line 3, col 7] Scan error: unterminated string");
    }

    #[test]
    fn compile_error_display() {
        let err = KontraError::compile(Span::new(10, 1, 100, 105), "expected '}'");
        let msg = err.to_string();
        assert_eq!(msg, "[line 10, col 1] Compile error: expected '}'");
    }

    #[test]
    fn runtime_error_with_line_display() {
        let err = KontraError::runtime(Some(42), "stack underflow");
        let msg = err.to_string();
        assert_eq!(msg, "[line 42] Runtime error: stack underflow");
    }

    #[test]
    fn runtime_error_without_line_display() {
        let err = KontraError::runtime(None, "unknown opcode");
        let msg = err.to_string();
        assert_eq!(msg, "Runtime error: unknown opcode");
    }
}
