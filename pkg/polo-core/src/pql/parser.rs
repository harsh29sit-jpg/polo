use super::{
    ast::{BinOp, Column, Direction, Expr, Literal, OrderBy, Query},
    token::Token,
};
use crate::error::Error;

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    pub fn parse(mut self) -> Result<Query, Error> {
        self.expect(Token::Select)?;

        let columns = self.parse_columns()?;

        self.expect(Token::From)?;
        let namespace = match self.advance() {
            Token::Ident(s) => s,
            other => {
                return Err(Error::Query(format!(
                    "expected namespace name after FROM, got {:?}",
                    other
                )))
            }
        };

        let mut filter = None;
        let mut effective_at = None;
        let mut asof = None;
        let mut order_by = Vec::new();
        let mut limit = None;
        let mut offset = None;

        loop {
            match self.peek() {
                Token::Where => {
                    self.advance();
                    filter = Some(self.parse_expr()?);
                }
                Token::EffectiveAt => {
                    self.advance();
                    effective_at = Some(self.parse_datetime()?);
                }
                Token::AsOf => {
                    self.advance();
                    asof = Some(self.parse_hlc()?);
                }
                Token::Order => {
                    self.advance(); // consumed ORDER (BY is part of the ORDER token from lexer)
                    order_by = self.parse_order_by()?;
                }
                Token::Limit => {
                    self.advance();
                    limit = Some(self.parse_usize("LIMIT")?);
                }
                Token::Offset => {
                    self.advance();
                    offset = Some(self.parse_usize("OFFSET")?);
                }
                Token::Eof => break,
                other => {
                    return Err(Error::Query(format!(
                        "unexpected token in query: {:?}",
                        other
                    )))
                }
            }
        }

        Ok(Query {
            columns,
            namespace,
            filter,
            effective_at,
            asof,
            order_by,
            limit,
            offset,
        })
    }

    fn parse_columns(&mut self) -> Result<Vec<Column>, Error> {
        let mut cols = Vec::new();
        loop {
            match self.peek() {
                Token::Star => {
                    self.advance();
                    cols.push(Column::Star);
                }
                Token::Ident(_) => {
                    if let Token::Ident(name) = self.advance() {
                        cols.push(Column::Named(name));
                    }
                }
                _ => break,
            }
            if self.peek() == Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        if cols.is_empty() {
            return Err(Error::Query("SELECT needs at least one column".into()));
        }
        Ok(cols)
    }

    fn parse_expr(&mut self) -> Result<Expr, Error> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, Error> {
        let mut left = self.parse_and()?;
        while self.peek() == Token::Or {
            self.advance();
            let right = self.parse_and()?;
            left = Expr::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr, Error> {
        let mut left = self.parse_not()?;
        while self.peek() == Token::And {
            self.advance();
            let right = self.parse_not()?;
            left = Expr::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_not(&mut self) -> Result<Expr, Error> {
        if self.peek() == Token::Not {
            self.advance();
            let inner = self.parse_not()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_predicate()
    }

    fn parse_predicate(&mut self) -> Result<Expr, Error> {
        let left = self.parse_atom()?;

        match self.peek() {
            Token::Eq => {
                self.advance();
                Ok(Expr::BinOp {
                    op: BinOp::Eq,
                    left: Box::new(left),
                    right: Box::new(self.parse_atom()?),
                })
            }
            Token::Ne => {
                self.advance();
                Ok(Expr::BinOp {
                    op: BinOp::Ne,
                    left: Box::new(left),
                    right: Box::new(self.parse_atom()?),
                })
            }
            Token::Lt => {
                self.advance();
                Ok(Expr::BinOp {
                    op: BinOp::Lt,
                    left: Box::new(left),
                    right: Box::new(self.parse_atom()?),
                })
            }
            Token::Le => {
                self.advance();
                Ok(Expr::BinOp {
                    op: BinOp::Le,
                    left: Box::new(left),
                    right: Box::new(self.parse_atom()?),
                })
            }
            Token::Gt => {
                self.advance();
                Ok(Expr::BinOp {
                    op: BinOp::Gt,
                    left: Box::new(left),
                    right: Box::new(self.parse_atom()?),
                })
            }
            Token::Ge => {
                self.advance();
                Ok(Expr::BinOp {
                    op: BinOp::Ge,
                    left: Box::new(left),
                    right: Box::new(self.parse_atom()?),
                })
            }
            Token::Like => {
                self.advance();
                match self.advance() {
                    Token::Str(pattern) => Ok(Expr::Like {
                        expr: Box::new(left),
                        pattern,
                    }),
                    other => Err(Error::Query(format!(
                        "LIKE expects a string pattern, got {:?}",
                        other
                    ))),
                }
            }
            Token::In => {
                self.advance();
                self.expect(Token::LParen)?;
                let mut values = Vec::new();
                loop {
                    values.push(self.parse_literal()?);
                    if self.peek() == Token::Comma {
                        self.advance();
                    } else {
                        break;
                    }
                }
                self.expect(Token::RParen)?;
                Ok(Expr::In {
                    expr: Box::new(left),
                    values,
                })
            }
            Token::Is => {
                self.advance();
                if self.peek() == Token::Not {
                    self.advance();
                    self.expect(Token::Null)?;
                    Ok(Expr::IsNotNull(Box::new(left)))
                } else {
                    self.expect(Token::Null)?;
                    Ok(Expr::IsNull(Box::new(left)))
                }
            }
            _ => Ok(left),
        }
    }

    fn parse_atom(&mut self) -> Result<Expr, Error> {
        match self.peek().clone() {
            Token::Ident(name) => {
                self.advance();
                Ok(Expr::Column(name))
            }
            Token::LParen => {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect(Token::RParen)?;
                Ok(inner)
            }
            _ => Ok(Expr::Lit(self.parse_literal()?)),
        }
    }

    fn parse_literal(&mut self) -> Result<Literal, Error> {
        match self.advance() {
            Token::Str(s) => Ok(Literal::Str(s)),
            Token::Int(n) => Ok(Literal::Int(n)),
            Token::Float(f) => Ok(Literal::Float(f)),
            Token::True => Ok(Literal::Bool(true)),
            Token::False => Ok(Literal::Bool(false)),
            Token::Null => Ok(Literal::Null),
            other => Err(Error::Query(format!("expected literal, got {:?}", other))),
        }
    }

    fn parse_order_by(&mut self) -> Result<Vec<OrderBy>, Error> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                Token::Ident(_) => {
                    if let Token::Ident(col) = self.advance() {
                        let dir = match self.peek() {
                            Token::Asc => {
                                self.advance();
                                Direction::Asc
                            }
                            Token::Desc => {
                                self.advance();
                                Direction::Desc
                            }
                            _ => Direction::Asc,
                        };
                        items.push(OrderBy {
                            column: col,
                            direction: dir,
                        });
                        if self.peek() == Token::Comma {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(items)
    }

    fn parse_datetime(&mut self) -> Result<chrono::DateTime<chrono::Utc>, Error> {
        match self.advance() {
            Token::Str(s) => s
                .parse::<chrono::DateTime<chrono::Utc>>()
                .map_err(|_| Error::Query(format!("invalid timestamp: {s}"))),
            other => Err(Error::Query(format!(
                "expected timestamp string, got {:?}",
                other
            ))),
        }
    }

    fn parse_hlc(&mut self) -> Result<crate::clock::Hlc, Error> {
        match self.advance() {
            Token::Str(s) => s
                .parse::<crate::clock::Hlc>()
                .map_err(|_| Error::Query(format!("invalid HLC: {s}"))),
            other => Err(Error::Query(format!("expected HLC string, got {:?}", other))),
        }
    }

    fn parse_usize(&mut self, ctx: &str) -> Result<usize, Error> {
        match self.advance() {
            Token::Int(n) if n >= 0 => Ok(n as usize),
            other => Err(Error::Query(format!(
                "{ctx} expects a non-negative integer, got {:?}",
                other
            ))),
        }
    }

    fn peek(&self) -> Token {
        self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof)
    }

    fn advance(&mut self) -> Token {
        let tok = self.tokens.get(self.pos).cloned().unwrap_or(Token::Eof);
        self.pos += 1;
        tok
    }

    fn expect(&mut self, expected: Token) -> Result<(), Error> {
        let got = self.advance();
        if got == expected {
            Ok(())
        } else {
            Err(Error::Query(format!("expected {:?}, got {:?}", expected, got)))
        }
    }
}
