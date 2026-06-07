#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // keywords
    Select,
    From,
    Where,
    And,
    Or,
    Not,
    Order,
    By,
    Asc,
    Desc,
    Limit,
    Offset,
    EffectiveAt,
    AsOf,
    Like,
    In,
    Is,
    Null,
    True,
    False,

    // literals
    Ident(String),
    Str(String),
    Int(i64),
    Float(f64),

    // operators
    Eq,   // =
    Ne,   // !=
    Lt,   // <
    Le,   // <=
    Gt,   // >
    Ge,   // >=
    Star, // *

    // punctuation
    Comma,
    Dot,
    LParen,
    RParen,

    Eof,
}

impl Token {
    pub fn keyword(s: &str) -> Option<Token> {
        match s.to_ascii_uppercase().as_str() {
            "SELECT" => Some(Token::Select),
            "FROM" => Some(Token::From),
            "WHERE" => Some(Token::Where),
            "AND" => Some(Token::And),
            "OR" => Some(Token::Or),
            "NOT" => Some(Token::Not),
            "ORDER" => Some(Token::Order),
            "BY" => Some(Token::By),
            "ASC" => Some(Token::Asc),
            "DESC" => Some(Token::Desc),
            "LIMIT" => Some(Token::Limit),
            "OFFSET" => Some(Token::Offset),
            "EFFECTIVE" => Some(Token::EffectiveAt), // consumed together with AT
            "AS" => Some(Token::AsOf),               // consumed together with OF
            "LIKE" => Some(Token::Like),
            "IN" => Some(Token::In),
            "IS" => Some(Token::Is),
            "NULL" => Some(Token::Null),
            "TRUE" => Some(Token::True),
            "FALSE" => Some(Token::False),
            _ => None,
        }
    }
}
