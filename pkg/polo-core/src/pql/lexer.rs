use super::token::Token;
use crate::error::Error;

pub struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    pub fn tokenize(mut self) -> Result<Vec<Token>, Error> {
        let mut tokens = Vec::new();
        loop {
            let tok = self.next_token()?;
            let is_eof = tok == Token::Eof;
            tokens.push(tok);
            if is_eof {
                break;
            }
        }
        Ok(tokens)
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn advance(&mut self) -> Option<u8> {
        let ch = self.input.get(self.pos).copied();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn read_ident(&mut self, first: u8) -> String {
        let mut s = vec![first];
        while matches!(self.peek(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'/')) {
            s.push(self.advance().unwrap());
        }
        String::from_utf8(s).unwrap()
    }

    fn read_string(&mut self) -> Result<String, Error> {
        let mut s = Vec::new();
        loop {
            match self.advance() {
                None => {
                    return Err(Error::Query("unterminated string literal".into()));
                }
                Some(b'\'') => break,
                Some(b'\\') => match self.advance() {
                    Some(b'\'') => s.push(b'\''),
                    Some(b'\\') => s.push(b'\\'),
                    Some(b'n') => s.push(b'\n'),
                    Some(b't') => s.push(b'\t'),
                    Some(c) => {
                        s.push(b'\\');
                        s.push(c);
                    }
                    None => return Err(Error::Query("unexpected end of string escape".into())),
                },
                Some(c) => s.push(c),
            }
        }
        String::from_utf8(s).map_err(|_| Error::Query("string is not valid UTF-8".into()))
    }

    fn read_number(&mut self, first: u8) -> Result<Token, Error> {
        let mut s = vec![first];
        let mut is_float = false;

        while let Some(c @ (b'0'..=b'9' | b'.' | b'e' | b'E' | b'-' | b'+')) = self.peek() {
            if c == b'.' || c == b'e' || c == b'E' {
                is_float = true;
            }
            s.push(c);
            self.pos += 1;
        }

        let raw = String::from_utf8(s).unwrap();
        if is_float {
            raw.parse::<f64>()
                .map(Token::Float)
                .map_err(|_| Error::Query(format!("invalid float literal: {raw}")))
        } else {
            raw.parse::<i64>()
                .map(Token::Int)
                .map_err(|_| Error::Query(format!("invalid integer literal: {raw}")))
        }
    }

    fn next_token(&mut self) -> Result<Token, Error> {
        self.skip_whitespace();

        match self.advance() {
            None => Ok(Token::Eof),
            Some(b'\'') => self.read_string().map(Token::Str),
            Some(b'*') => Ok(Token::Star),
            Some(b',') => Ok(Token::Comma),
            Some(b'.') => Ok(Token::Dot),
            Some(b'(') => Ok(Token::LParen),
            Some(b')') => Ok(Token::RParen),
            Some(b'=') => Ok(Token::Eq),
            Some(b'!') => {
                if self.peek() == Some(b'=') {
                    self.pos += 1;
                    Ok(Token::Ne)
                } else {
                    Err(Error::Query("unexpected '!'".into()))
                }
            }
            Some(b'<') => {
                if self.peek() == Some(b'=') {
                    self.pos += 1;
                    Ok(Token::Le)
                } else {
                    Ok(Token::Lt)
                }
            }
            Some(b'>') => {
                if self.peek() == Some(b'=') {
                    self.pos += 1;
                    Ok(Token::Ge)
                } else {
                    Ok(Token::Gt)
                }
            }
            Some(c @ (b'0'..=b'9')) => self.read_number(c),
            Some(b'-') => {
                // could be a negative number
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.read_number(b'-')
                } else {
                    Err(Error::Query("unexpected '-'".into()))
                }
            }
            Some(c @ (b'a'..=b'z' | b'A'..=b'Z' | b'_')) => {
                let ident = self.read_ident(c);
                // multi-word keywords
                if ident.to_ascii_uppercase() == "EFFECTIVE" {
                    self.skip_whitespace();
                    let next = self.read_ident_if_keyword("AT")?;
                    if next {
                        return Ok(Token::EffectiveAt);
                    }
                } else if ident.to_ascii_uppercase() == "AS" {
                    self.skip_whitespace();
                    let next = self.read_ident_if_keyword("OF")?;
                    if next {
                        return Ok(Token::AsOf);
                    }
                } else if ident.to_ascii_uppercase() == "ORDER" {
                    self.skip_whitespace();
                    let _ = self.read_ident_if_keyword("BY")?;
                    return Ok(Token::Order);
                }
                if let Some(kw) = Token::keyword(&ident) {
                    Ok(kw)
                } else {
                    Ok(Token::Ident(ident))
                }
            }
            Some(c) => Err(Error::Query(format!("unexpected character '{}'", c as char))),
        }
    }

    fn read_ident_if_keyword(&mut self, expected: &str) -> Result<bool, Error> {
        let start = self.pos;
        match self.peek() {
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'_') => {
                let ch = self.advance().unwrap();
                let word = self.read_ident(ch);
                if word.to_ascii_uppercase() == expected {
                    Ok(true)
                } else {
                    self.pos = start;
                    Ok(false)
                }
            }
            _ => Ok(false),
        }
    }
}
