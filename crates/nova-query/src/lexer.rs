use nova_core::error::{NovaError, Result};
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub line: usize,
    pub column: usize,
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}", self.line, self.column)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Keywords
    Find,
    Insert,
    Into,
    Values,
    Update,
    Set,
    Remove,
    Watch,
    Count,
    Exists,
    Create,
    Drop,
    Index,
    Type,
    Where,
    Sort,
    Limit,
    Offset,
    Asc,
    Desc,
    And,
    Or,
    Not,
    In,
    Between,
    Begin,
    Commit,
    Rollback,
    Hash,
    Ordered,
    True,
    False,
    Null,

    // Literals
    Int(i64),
    Float(f64),
    StringLit(String),
    Identifier(String),

    // Symbols
    EqEq,     // ==
    Eq,       // =
    BangEq,   // !=
    Gt,       // >
    GtEq,     // >=
    Lt,       // <
    LtEq,     // <=
    AmpAmp,   // &&
    PipePipe, // ||
    Bang,     // !
    Plus,     // +
    Minus,    // -
    Star,     // *
    Slash,    // /
    Percent,  // %
    LParen,   // (
    RParen,   // )
    LBrace,   // {
    RBrace,   // }
    LBracket, // [
    RBracket, // ]
    Comma,    // ,
    Colon,    // :
    Dot,      // .
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

pub struct Lexer<'a> {
    chars: Vec<(usize, usize, char)>, // (line, col, char)
    cursor: usize,
    _source: &'a str,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut chars = Vec::new();
        let mut line = 1;
        let mut col = 1;

        for ch in source.chars() {
            chars.push((line, col, ch));
            if ch == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }

        Self {
            chars,
            cursor: 0,
            _source: source,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.cursor).map(|(_, _, c)| *c)
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.cursor + 1).map(|(_, _, c)| *c)
    }

    fn current_span(&self) -> Span {
        if let Some(&(l, c, _)) = self.chars.get(self.cursor) {
            Span { line: l, column: c }
        } else if let Some(&(l, c, _)) = self.chars.last() {
            Span {
                line: l,
                column: c + 1,
            }
        } else {
            Span { line: 1, column: 1 }
        }
    }

    fn advance(&mut self) -> Option<char> {
        if self.cursor < self.chars.len() {
            let ch = self.chars[self.cursor].2;
            self.cursor += 1;
            Some(ch)
        } else {
            None
        }
    }

    pub fn tokenize(&mut self) -> Result<Vec<Token>> {
        let mut tokens = Vec::new();

        while let Some(ch) = self.peek() {
            let span = self.current_span();

            // Whitespace
            if ch.is_whitespace() {
                self.advance();
                continue;
            }

            // Comments (-- or //)
            if (ch == '-' && self.peek_next() == Some('-'))
                || (ch == '/' && self.peek_next() == Some('/'))
            {
                while let Some(c) = self.advance() {
                    if c == '\n' {
                        break;
                    }
                }
                continue;
            }

            // String literals ("..." or '...')
            if ch == '"' || ch == '\'' {
                let quote = self.advance().unwrap();
                let mut content = String::new();
                let mut closed = false;

                while let Some(c) = self.advance() {
                    if c == quote {
                        closed = true;
                        break;
                    }
                    if c == '\\' {
                        if let Some(escaped) = self.advance() {
                            match escaped {
                                'n' => content.push('\n'),
                                't' => content.push('\t'),
                                'r' => content.push('\r'),
                                '\\' => content.push('\\'),
                                '"' => content.push('"'),
                                '\'' => content.push('\''),
                                other => content.push(other),
                            }
                            continue;
                        }
                    }
                    content.push(c);
                }

                if !closed {
                    return Err(NovaError::parse_error(
                        "Unterminated string literal",
                        span.line,
                        span.column,
                    ));
                }

                tokens.push(Token {
                    kind: TokenKind::StringLit(content),
                    span,
                });
                continue;
            }

            // Numbers (integers or floats)
            if ch.is_ascii_digit()
                || (ch == '-'
                    && self
                        .peek_next()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false))
            {
                let mut num_str = String::new();
                if ch == '-' {
                    num_str.push(self.advance().unwrap());
                }
                let mut has_dot = false;

                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        num_str.push(self.advance().unwrap());
                    } else if c == '.'
                        && !has_dot
                        && self
                            .peek_next()
                            .map(|n| n.is_ascii_digit())
                            .unwrap_or(false)
                    {
                        has_dot = true;
                        num_str.push(self.advance().unwrap());
                    } else {
                        break;
                    }
                }

                if has_dot {
                    let f: f64 = num_str.parse().map_err(|_| {
                        NovaError::parse_error(
                            format!("Invalid float literal '{num_str}'"),
                            span.line,
                            span.column,
                        )
                    })?;
                    tokens.push(Token {
                        kind: TokenKind::Float(f),
                        span,
                    });
                } else {
                    let i: i64 = num_str.parse().map_err(|_| {
                        NovaError::parse_error(
                            format!("Invalid integer literal '{num_str}'"),
                            span.line,
                            span.column,
                        )
                    })?;
                    tokens.push(Token {
                        kind: TokenKind::Int(i),
                        span,
                    });
                }
                continue;
            }

            // Identifiers and Keywords
            if ch.is_alphabetic() || ch == '_' {
                let mut ident = String::new();
                while let Some(c) = self.peek() {
                    if c.is_alphanumeric() || c == '_' {
                        ident.push(self.advance().unwrap());
                    } else {
                        break;
                    }
                }

                let upper = ident.to_ascii_uppercase();
                let kind = match upper.as_str() {
                    "FIND" => TokenKind::Find,
                    "INSERT" => TokenKind::Insert,
                    "INTO" => TokenKind::Into,
                    "VALUES" => TokenKind::Values,
                    "UPDATE" => TokenKind::Update,
                    "SET" => TokenKind::Set,
                    "REMOVE" => TokenKind::Remove,
                    "WATCH" => TokenKind::Watch,
                    "COUNT" => TokenKind::Count,
                    "EXISTS" => TokenKind::Exists,
                    "CREATE" => TokenKind::Create,
                    "DROP" => TokenKind::Drop,
                    "INDEX" => TokenKind::Index,
                    "TYPE" => TokenKind::Type,
                    "WHERE" => TokenKind::Where,
                    "SORT" => TokenKind::Sort,
                    "LIMIT" => TokenKind::Limit,
                    "OFFSET" => TokenKind::Offset,
                    "ASC" => TokenKind::Asc,
                    "DESC" => TokenKind::Desc,
                    "AND" => TokenKind::And,
                    "OR" => TokenKind::Or,
                    "NOT" => TokenKind::Not,
                    "IN" => TokenKind::In,
                    "BETWEEN" => TokenKind::Between,
                    "BEGIN" => TokenKind::Begin,
                    "COMMIT" => TokenKind::Commit,
                    "ROLLBACK" => TokenKind::Rollback,
                    "HASH" => TokenKind::Hash,
                    "ORDERED" => TokenKind::Ordered,
                    "TRUE" => TokenKind::True,
                    "FALSE" => TokenKind::False,
                    "NULL" => TokenKind::Null,
                    _ => TokenKind::Identifier(ident),
                };

                tokens.push(Token { kind, span });
                continue;
            }

            // Punctuation and Multi-character Symbols
            match ch {
                '=' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token {
                            kind: TokenKind::EqEq,
                            span,
                        });
                    } else {
                        tokens.push(Token {
                            kind: TokenKind::Eq,
                            span,
                        });
                    }
                }
                '!' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token {
                            kind: TokenKind::BangEq,
                            span,
                        });
                    } else {
                        tokens.push(Token {
                            kind: TokenKind::Bang,
                            span,
                        });
                    }
                }
                '>' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token {
                            kind: TokenKind::GtEq,
                            span,
                        });
                    } else {
                        tokens.push(Token {
                            kind: TokenKind::Gt,
                            span,
                        });
                    }
                }
                '<' => {
                    self.advance();
                    if self.peek() == Some('=') {
                        self.advance();
                        tokens.push(Token {
                            kind: TokenKind::LtEq,
                            span,
                        });
                    } else {
                        tokens.push(Token {
                            kind: TokenKind::Lt,
                            span,
                        });
                    }
                }
                '&' => {
                    self.advance();
                    if self.peek() == Some('&') {
                        self.advance();
                        tokens.push(Token {
                            kind: TokenKind::AmpAmp,
                            span,
                        });
                    } else {
                        return Err(NovaError::parse_error(
                            "Unexpected '&', expected '&&'",
                            span.line,
                            span.column,
                        ));
                    }
                }
                '|' => {
                    self.advance();
                    if self.peek() == Some('|') {
                        self.advance();
                        tokens.push(Token {
                            kind: TokenKind::PipePipe,
                            span,
                        });
                    } else {
                        return Err(NovaError::parse_error(
                            "Unexpected '|', expected '||'",
                            span.line,
                            span.column,
                        ));
                    }
                }
                '+' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Plus,
                        span,
                    });
                }
                '-' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Minus,
                        span,
                    });
                }
                '*' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Star,
                        span,
                    });
                }
                '/' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Slash,
                        span,
                    });
                }
                '%' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Percent,
                        span,
                    });
                }
                '(' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::LParen,
                        span,
                    });
                }
                ')' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::RParen,
                        span,
                    });
                }
                '{' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::LBrace,
                        span,
                    });
                }
                '}' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::RBrace,
                        span,
                    });
                }
                '[' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::LBracket,
                        span,
                    });
                }
                ']' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::RBracket,
                        span,
                    });
                }
                ',' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Comma,
                        span,
                    });
                }
                ':' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Colon,
                        span,
                    });
                }
                '.' => {
                    self.advance();
                    tokens.push(Token {
                        kind: TokenKind::Dot,
                        span,
                    });
                }
                unexpected => {
                    return Err(NovaError::parse_error(
                        format!("Unexpected character: '{unexpected}'"),
                        span.line,
                        span.column,
                    ));
                }
            }
        }

        tokens.push(Token {
            kind: TokenKind::Eof,
            span: self.current_span(),
        });

        Ok(tokens)
    }
}
