use crate::ast::{Assignment, BinaryOp, Expr, SortClause, SortDirection, Statement, UnaryOp};
use crate::lexer::{Span, Token, TokenKind};
use nova_core::document::Document;
use nova_core::error::{NovaError, Result};
use nova_core::value::Value;
use nova_index::IndexType;
use std::collections::BTreeMap;

pub struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, cursor: 0 }
    }

    fn peek(&self) -> &Token {
        self.tokens
            .get(self.cursor)
            .unwrap_or_else(|| self.tokens.last().unwrap())
    }

    fn peek_kind(&self) -> &TokenKind {
        &self.peek().kind
    }

    fn advance(&mut self) -> &Token {
        if self.cursor < self.tokens.len() - 1 {
            let tok = &self.tokens[self.cursor];
            self.cursor += 1;
            tok
        } else {
            &self.tokens[self.cursor]
        }
    }

    fn current_span(&self) -> Span {
        self.peek().span
    }

    fn check(&self, kind: &TokenKind) -> bool {
        self.peek_kind() == kind
    }

    fn match_token(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, kind: TokenKind, msg: &str) -> Result<Token> {
        if self.check(&kind) {
            Ok(self.advance().clone())
        } else {
            let span = self.current_span();
            Err(NovaError::parse_error(
                format!("{msg} (found '{:?}')", self.peek_kind()),
                span.line,
                span.column,
            ))
        }
    }

    fn expect_identifier(&mut self, msg: &str) -> Result<String> {
        let span = self.current_span();
        match self.peek_kind() {
            TokenKind::Identifier(s) => {
                let name = s.clone();
                self.advance();
                Ok(name)
            }
            other => Err(NovaError::parse_error(
                format!("{msg}, found '{:?}'", other),
                span.line,
                span.column,
            )),
        }
    }

    /// Parse complete statement.
    pub fn parse_statement(&mut self) -> Result<Statement> {
        match self.peek_kind() {
            TokenKind::Find => self.parse_find(),
            TokenKind::Insert => self.parse_insert(),
            TokenKind::Update => self.parse_update(),
            TokenKind::Remove => self.parse_remove(),
            TokenKind::Watch => self.parse_watch(),
            TokenKind::Count => self.parse_count(),
            TokenKind::Exists => self.parse_exists(),
            TokenKind::Create => self.parse_create_index(),
            TokenKind::Drop => self.parse_drop_index(),
            TokenKind::Begin => {
                self.advance();
                Ok(Statement::Begin)
            }
            TokenKind::Commit => {
                self.advance();
                Ok(Statement::Commit)
            }
            TokenKind::Rollback => {
                self.advance();
                Ok(Statement::Rollback)
            }
            other => {
                let span = self.current_span();
                Err(NovaError::parse_error(
                    format!("Expected NQL command (FIND, INSERT, UPDATE, REMOVE, WATCH, etc.), found '{:?}'", other),
                    span.line,
                    span.column,
                ))
            }
        }
    }

    // FIND users [WHERE expr] [SORT field [ASC|DESC]] [LIMIT n] [OFFSET n]
    fn parse_find(&mut self) -> Result<Statement> {
        self.advance(); // FIND
        let collection = self.expect_identifier("Expected collection name after FIND")?;

        let filter = if self.match_token(&TokenKind::Where) {
            Some(self.parse_expr()?)
        } else {
            None
        };

        let sort = if self.match_token(&TokenKind::Sort) {
            let field = self.parse_field_path()?;
            let direction = if self.match_token(&TokenKind::Desc) {
                SortDirection::Desc
            } else {
                self.match_token(&TokenKind::Asc);
                SortDirection::Asc
            };
            Some(SortClause { field, direction })
        } else {
            None
        };

        let limit = if self.match_token(&TokenKind::Limit) {
            Some(self.parse_usize("Expected integer after LIMIT")?)
        } else {
            None
        };

        let offset = if self.match_token(&TokenKind::Offset) {
            Some(self.parse_usize("Expected integer after OFFSET")?)
        } else {
            None
        };

        Ok(Statement::Find {
            collection,
            filter,
            sort,
            limit,
            offset,
        })
    }

    // INSERT [INTO] users VALUES ({ ... })
    fn parse_insert(&mut self) -> Result<Statement> {
        self.advance(); // INSERT
        self.match_token(&TokenKind::Into);
        let collection = self.expect_identifier("Expected collection name after INSERT")?;

        self.expect(TokenKind::Values, "Expected VALUES keyword")?;

        let has_paren = self.match_token(&TokenKind::LParen);
        let value = self.parse_value_literal()?;
        if has_paren {
            self.expect(TokenKind::RParen, "Expected ')' closing VALUES")?;
        }

        let document = match value {
            Value::Object(map) => {
                let mut doc = Document::new();
                for (k, v) in map {
                    if k == "_id" || k == "id" {
                        if let Ok(id_str) = v.as_str() {
                            if let Ok(doc_id) = nova_core::document::DocumentId::new(id_str) {
                                doc.id = doc_id;
                            }
                        }
                    } else {
                        doc.insert(k, v);
                    }
                }
                doc
            }
            other => {
                let span = self.current_span();
                return Err(NovaError::parse_error(
                    format!(
                        "INSERT document must be an object, found '{}'",
                        other.type_name()
                    ),
                    span.line,
                    span.column,
                ));
            }
        };

        Ok(Statement::Insert {
            collection,
            document,
        })
    }

    // UPDATE users SET role = "engineer", age = 29 [WHERE id == "u1"]
    fn parse_update(&mut self) -> Result<Statement> {
        self.advance(); // UPDATE
        let collection = self.expect_identifier("Expected collection name after UPDATE")?;

        self.expect(TokenKind::Set, "Expected SET keyword in UPDATE")?;

        let mut assignments = Vec::new();
        loop {
            let field = self.parse_field_path()?;
            if !self.match_token(&TokenKind::Eq) && !self.match_token(&TokenKind::EqEq) {
                let span = self.current_span();
                return Err(NovaError::parse_error(
                    "Expected '=' in SET assignment",
                    span.line,
                    span.column,
                ));
            }
            let expr = self.parse_expr()?;
            assignments.push(Assignment { field, expr });

            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }

        let filter = if self.match_token(&TokenKind::Where) {
            Some(self.parse_expr()?)
        } else {
            None
        };

        Ok(Statement::Update {
            collection,
            assignments,
            filter,
        })
    }

    // REMOVE users [WHERE expr]
    fn parse_remove(&mut self) -> Result<Statement> {
        self.advance(); // REMOVE
        let collection = self.expect_identifier("Expected collection name after REMOVE")?;

        let filter = if self.match_token(&TokenKind::Where) {
            Some(self.parse_expr()?)
        } else {
            None
        };

        Ok(Statement::Remove { collection, filter })
    }

    // WATCH users [WHERE expr]
    fn parse_watch(&mut self) -> Result<Statement> {
        self.advance(); // WATCH
        let collection = self.expect_identifier("Expected collection name after WATCH")?;

        let filter = if self.match_token(&TokenKind::Where) {
            Some(self.parse_expr()?)
        } else {
            None
        };

        Ok(Statement::Watch { collection, filter })
    }

    // COUNT users [WHERE expr]
    fn parse_count(&mut self) -> Result<Statement> {
        self.advance(); // COUNT
        let collection = self.expect_identifier("Expected collection name after COUNT")?;

        let filter = if self.match_token(&TokenKind::Where) {
            Some(self.parse_expr()?)
        } else {
            None
        };

        Ok(Statement::Count { collection, filter })
    }

    // EXISTS users [WHERE expr]
    fn parse_exists(&mut self) -> Result<Statement> {
        self.advance(); // EXISTS
        let collection = self.expect_identifier("Expected collection name after EXISTS")?;

        let filter = if self.match_token(&TokenKind::Where) {
            Some(self.parse_expr()?)
        } else {
            None
        };

        Ok(Statement::Exists { collection, filter })
    }

    // CREATE INDEX users.email [TYPE ordered|hash]
    fn parse_create_index(&mut self) -> Result<Statement> {
        self.advance(); // CREATE
        self.expect(TokenKind::Index, "Expected INDEX after CREATE")?;

        let target = self.expect_identifier("Expected collection name or target for index")?;
        let (collection, field) = if self.match_token(&TokenKind::Dot) {
            let field = self.parse_field_path()?;
            (target, field)
        } else {
            let field = self.expect_identifier("Expected field name after collection")?;
            (target, field)
        };

        let index_type = if self.match_token(&TokenKind::Type) {
            if self.match_token(&TokenKind::Hash) {
                IndexType::Hash
            } else if self.match_token(&TokenKind::Ordered) {
                IndexType::Ordered
            } else {
                let span = self.current_span();
                return Err(NovaError::parse_error(
                    "Index TYPE must be 'hash' or 'ordered'",
                    span.line,
                    span.column,
                ));
            }
        } else {
            IndexType::Ordered // Default to ordered
        };

        Ok(Statement::CreateIndex {
            collection,
            field,
            index_type,
        })
    }

    // DROP INDEX users.email
    fn parse_drop_index(&mut self) -> Result<Statement> {
        self.advance(); // DROP
        self.expect(TokenKind::Index, "Expected INDEX after DROP")?;

        let target = self.expect_identifier("Expected collection name or target for index")?;
        let (collection, field) = if self.match_token(&TokenKind::Dot) {
            let field = self.parse_field_path()?;
            (target, field)
        } else {
            let field = self.expect_identifier("Expected field name after collection")?;
            (target, field)
        };

        Ok(Statement::DropIndex { collection, field })
    }

    fn parse_usize(&mut self, msg: &str) -> Result<usize> {
        let span = self.current_span();
        match self.peek_kind() {
            TokenKind::Int(i) if *i >= 0 => {
                let val = *i as usize;
                self.advance();
                Ok(val)
            }
            _ => Err(NovaError::parse_error(msg, span.line, span.column)),
        }
    }

    fn parse_field_path(&mut self) -> Result<String> {
        let mut path = self.expect_identifier("Expected field name")?;
        while self.match_token(&TokenKind::Dot) {
            let sub = self.expect_identifier("Expected nested field identifier after '.'")?;
            path.push('.');
            path.push_str(&sub);
        }
        Ok(path)
    }

    // Expression parsing with operator precedence (Pratt parsing)
    pub fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut left = self.parse_and()?;
        while self.match_token(&TokenKind::Or) || self.match_token(&TokenKind::PipePipe) {
            let right = self.parse_and()?;
            left = Expr::Binary {
                op: BinaryOp::Or,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut left = self.parse_comparison()?;
        while self.match_token(&TokenKind::And) || self.match_token(&TokenKind::AmpAmp) {
            let right = self.parse_comparison()?;
            left = Expr::Binary {
                op: BinaryOp::And,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_comparison(&mut self) -> Result<Expr> {
        let left = self.parse_additive()?;

        if self.match_token(&TokenKind::EqEq) || self.match_token(&TokenKind::Eq) {
            let right = self.parse_additive()?;
            return Ok(Expr::Binary {
                op: BinaryOp::Eq,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        if self.match_token(&TokenKind::BangEq) {
            let right = self.parse_additive()?;
            return Ok(Expr::Binary {
                op: BinaryOp::NotEq,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        if self.match_token(&TokenKind::Gt) {
            let right = self.parse_additive()?;
            return Ok(Expr::Binary {
                op: BinaryOp::Gt,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        if self.match_token(&TokenKind::GtEq) {
            let right = self.parse_additive()?;
            return Ok(Expr::Binary {
                op: BinaryOp::GtEq,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        if self.match_token(&TokenKind::Lt) {
            let right = self.parse_additive()?;
            return Ok(Expr::Binary {
                op: BinaryOp::Lt,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        if self.match_token(&TokenKind::LtEq) {
            let right = self.parse_additive()?;
            return Ok(Expr::Binary {
                op: BinaryOp::LtEq,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        if self.match_token(&TokenKind::Between) {
            let min = self.parse_additive()?;
            self.expect(TokenKind::And, "Expected AND in BETWEEN expression")?;
            let max = self.parse_additive()?;
            return Ok(Expr::Between {
                expr: Box::new(left),
                min: Box::new(min),
                max: Box::new(max),
            });
        }
        if self.match_token(&TokenKind::In) {
            self.expect(TokenKind::LParen, "Expected '(' after IN")?;
            let mut list = Vec::new();
            if !self.check(&TokenKind::RParen) {
                loop {
                    list.push(self.parse_expr()?);
                    if !self.match_token(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(TokenKind::RParen, "Expected ')' closing IN list")?;
            return Ok(Expr::In {
                expr: Box::new(left),
                list,
            });
        }

        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr> {
        let mut left = self.parse_multiplicative()?;
        while self.check(&TokenKind::Plus) || self.check(&TokenKind::Minus) {
            let op = if self.match_token(&TokenKind::Plus) {
                BinaryOp::Plus
            } else {
                self.advance();
                BinaryOp::Minus
            };
            let right = self.parse_multiplicative()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr> {
        let mut left = self.parse_unary()?;
        while self.check(&TokenKind::Star)
            || self.check(&TokenKind::Slash)
            || self.check(&TokenKind::Percent)
        {
            let op = if self.match_token(&TokenKind::Star) {
                BinaryOp::Multiply
            } else if self.match_token(&TokenKind::Slash) {
                BinaryOp::Divide
            } else {
                self.advance();
                BinaryOp::Modulo
            };
            let right = self.parse_unary()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        if self.match_token(&TokenKind::Not) || self.match_token(&TokenKind::Bang) {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(expr),
            });
        }
        if self.match_token(&TokenKind::Minus) {
            let expr = self.parse_unary()?;
            return Ok(Expr::Unary {
                op: UnaryOp::Negate,
                expr: Box::new(expr),
            });
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let span = self.current_span();
        match self.peek_kind() {
            TokenKind::Null => {
                self.advance();
                Ok(Expr::Literal(Value::Null))
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr::Literal(Value::Bool(true)))
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr::Literal(Value::Bool(false)))
            }
            TokenKind::Int(i) => {
                let val = *i;
                self.advance();
                Ok(Expr::Literal(Value::Int(val)))
            }
            TokenKind::Float(f) => {
                let val = *f;
                self.advance();
                Ok(Expr::Literal(Value::Float(val)))
            }
            TokenKind::StringLit(s) => {
                let val = s.clone();
                self.advance();
                Ok(Expr::Literal(Value::String(val)))
            }
            TokenKind::Identifier(_) => {
                let path = self.parse_field_path()?;
                Ok(Expr::Field(path))
            }
            TokenKind::LParen => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(TokenKind::RParen, "Expected ')' closing expression")?;
                Ok(expr)
            }
            TokenKind::LBrace | TokenKind::LBracket => {
                let val = self.parse_value_literal()?;
                Ok(Expr::Literal(val))
            }
            other => Err(NovaError::parse_error(
                format!("Expected expression, found '{:?}'", other),
                span.line,
                span.column,
            )),
        }
    }

    /// Parse JSON/NQL literal value (objects, arrays, primitives)
    fn parse_value_literal(&mut self) -> Result<Value> {
        let span = self.current_span();
        match self.peek_kind() {
            TokenKind::Null => {
                self.advance();
                Ok(Value::Null)
            }
            TokenKind::True => {
                self.advance();
                Ok(Value::Bool(true))
            }
            TokenKind::False => {
                self.advance();
                Ok(Value::Bool(false))
            }
            TokenKind::Int(i) => {
                let val = *i;
                self.advance();
                Ok(Value::Int(val))
            }
            TokenKind::Float(f) => {
                let val = *f;
                self.advance();
                Ok(Value::Float(val))
            }
            TokenKind::StringLit(s) => {
                let val = s.clone();
                self.advance();
                Ok(Value::String(val))
            }
            TokenKind::LBracket => {
                self.advance();
                let mut list = Vec::new();
                if !self.check(&TokenKind::RBracket) {
                    loop {
                        list.push(self.parse_value_literal()?);
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RBracket, "Expected ']' closing array")?;
                Ok(Value::Array(list))
            }
            TokenKind::LBrace => {
                self.advance();
                let mut map = BTreeMap::new();
                if !self.check(&TokenKind::RBrace) {
                    loop {
                        let key = match self.peek_kind() {
                            TokenKind::StringLit(s) => {
                                let k = s.clone();
                                self.advance();
                                k
                            }
                            TokenKind::Identifier(s) => {
                                let k = s.clone();
                                self.advance();
                                k
                            }
                            other => {
                                let span = self.current_span();
                                return Err(NovaError::parse_error(
                                    format!(
                                        "Expected object key string or identifier, found '{:?}'",
                                        other
                                    ),
                                    span.line,
                                    span.column,
                                ));
                            }
                        };

                        self.expect(TokenKind::Colon, "Expected ':' after object key")?;
                        let val = self.parse_value_literal()?;
                        map.insert(key, val);

                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(TokenKind::RBrace, "Expected '}' closing object")?;
                Ok(Value::Object(map))
            }
            other => Err(NovaError::parse_error(
                format!("Expected value literal, found '{:?}'", other),
                span.line,
                span.column,
            )),
        }
    }
}
