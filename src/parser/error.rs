use thiserror::Error;

#[derive(Debug, Error, PartialEq, Clone)]
pub enum ParseError {
    #[error("unknown escape sequence: \\{0}")]
    UnknownEscape(char),
}
