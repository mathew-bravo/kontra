use std::fmt::Write as _;

use chrono::NaiveDate;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Num(f64),
    Date(NaiveDate),
    Identifier(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum OpCode {
    Constant,
    DefineParty,
    DefineEvent,
    DefineTerm,
    BeginObligation,
    SetParty,
    SetAction,
    SetDue,
    ConditionAfter,
    ConditionSatisfied,
    ConditionOccurred,
    ConditionAnd,
    SetCondition,
    EndObligation,
    BeginRemedy,
    EndRemedy,
    BeginPhase,
    EndPhase,
    Return,
    ConditionBefore,
    ConditionOr,
}

impl From<u8> for OpCode {
    fn from(byte: u8) -> Self {
        match byte {
            0 => OpCode::Constant,
            1 => OpCode::DefineParty,
            2 => OpCode::DefineEvent,
            3 => OpCode::DefineTerm,
            4 => OpCode::BeginObligation,
            5 => OpCode::SetParty,
            6 => OpCode::SetAction,
            7 => OpCode::SetDue,
            8 => OpCode::ConditionAfter,
            9 => OpCode::ConditionSatisfied,
            10 => OpCode::ConditionOccurred,
            11 => OpCode::ConditionAnd,
            12 => OpCode::SetCondition,
            13 => OpCode::EndObligation,
            14 => OpCode::BeginRemedy,
            15 => OpCode::EndRemedy,
            16 => OpCode::BeginPhase,
            17 => OpCode::EndPhase,
            18 => OpCode::Return,
            19 => OpCode::ConditionBefore,
            20 => OpCode::ConditionOr,
            _ => panic!("Unknown opcode: {}", byte),
        }
    }
}

impl From<OpCode> for u8 {
    fn from(op: OpCode) -> Self {
        op as u8
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Str(s) => write!(f, "\"{}\"", s),
            Value::Num(n) => write!(f, "{}", n),
            Value::Date(d) => write!(f, "{}", d),
            Value::Identifier(id) => write!(f, "{}", id),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Chunk {
    pub code: Vec<u8>,
    pub constants: Vec<Value>,
    pub lines: Vec<usize>,
}

impl Chunk {
    pub fn new() -> Self {
        Self {
            code: Vec::new(),
            constants: Vec::new(),
            lines: Vec::new(),
        }
    }

    pub fn write(&mut self, byte: u8, line: usize) {
        self.code.push(byte);
        self.lines.push(line);
    }

    pub fn add_constant(&mut self, value: Value) -> u8 {
        self.constants.push(value);
        (self.constants.len() - 1) as u8
    }

    pub fn disassemble(&self, name: &str) {
        print!("{}", self.disassemble_to_string(name));
    }

    /// Returns the disassembly as a String (useful for testing).
    pub fn disassemble_to_string(&self, name: &str) -> String {
        let mut out = String::new();
        writeln!(out, "== {} ==", name).unwrap();
        let mut offset = 0;
        while offset < self.code.len() {
            offset = self.disassemble_instruction_to(&mut out, offset);
        }
        out
    }

    fn disassemble_instruction_to(&self, out: &mut String, offset: usize) -> usize {
        write!(out, "{:04} ", offset).unwrap();

        // show line number, or | if same as previous
        if offset > 0 && self.lines[offset] == self.lines[offset - 1] {
            write!(out, "   | ").unwrap();
        } else {
            write!(out, "{:4} ", self.lines[offset]).unwrap();
        }

        let byte = self.code[offset];
        let op = OpCode::from(byte);

        match op {
            OpCode::Constant => {
                let idx = self.code[offset + 1] as usize;
                writeln!(
                    out,
                    "{:<20} {:4} '{}'",
                    "Constant", idx, self.constants[idx]
                )
                .unwrap();
                offset + 2
            }
            OpCode::Return => {
                writeln!(out, "Return").unwrap();
                offset + 1
            }
            // all other opcodes are simple (no operand)
            _ => {
                writeln!(out, "{:?}", op).unwrap();
                offset + 1
            }
        }
    }
}

impl Default for Chunk {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_write_and_read_back() {
        let mut chunk = Chunk::new();
        chunk.write(u8::from(OpCode::Return), 1);

        assert_eq!(chunk.code.len(), 1);
        assert_eq!(OpCode::from(chunk.code[0]), OpCode::Return);
        assert_eq!(chunk.lines[0], 1);
    }

    #[test]
    fn add_constant_returns_index() {
        let mut chunk = Chunk::new();
        let idx0 = chunk.add_constant(Value::Str("hello".into()));
        let idx1 = chunk.add_constant(Value::Num(42.0));

        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);
        assert_eq!(chunk.constants[0], Value::Str("hello".into()));
        assert_eq!(chunk.constants[1], Value::Num(42.0));
    }

    #[test]
    fn opcode_u8_roundtrip() {
        let ops = [
            OpCode::Constant,
            OpCode::DefineParty,
            OpCode::DefineEvent,
            OpCode::DefineTerm,
            OpCode::BeginObligation,
            OpCode::SetParty,
            OpCode::SetAction,
            OpCode::SetDue,
            OpCode::ConditionAfter,
            OpCode::ConditionSatisfied,
            OpCode::ConditionOccurred,
            OpCode::ConditionAnd,
            OpCode::SetCondition,
            OpCode::EndObligation,
            OpCode::BeginRemedy,
            OpCode::EndRemedy,
            OpCode::BeginPhase,
            OpCode::EndPhase,
            OpCode::Return,
            OpCode::ConditionBefore,
            OpCode::ConditionOr,
        ];
        for op in ops {
            let byte: u8 = op.into();
            let back = OpCode::from(byte);
            assert_eq!(back, op);
        }
    }

    #[test]
    fn disassemble_constant_and_return() {
        let mut chunk = Chunk::new();

        let idx = chunk.add_constant(Value::Str("Alice".into()));
        chunk.write(u8::from(OpCode::Constant), 1);
        chunk.write(idx, 1);

        let idx2 = chunk.add_constant(Value::Num(30.0));
        chunk.write(u8::from(OpCode::Constant), 2);
        chunk.write(idx2, 2);

        chunk.write(u8::from(OpCode::DefineParty), 2);
        chunk.write(u8::from(OpCode::Return), 3);

        let output = chunk.disassemble_to_string("test");

        assert!(output.contains("== test =="));
        assert!(output.contains("Constant"));
        assert!(output.contains("'\"Alice\"'"));
        assert!(output.contains("'30'"));
        assert!(output.contains("DefineParty"));
        assert!(output.contains("Return"));

        // second Constant and DefineParty share line 2, so | should appear
        assert!(output.contains("   | "));
    }
}
