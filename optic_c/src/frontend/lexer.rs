use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    EndOfFile,
    Keyword,
    Identifier,
    NumericConstant,
    StringLiteral,
    Punctuator,
    Preprocessor,
    Comment,
    WhiteSpace,
}

#[derive(Debug, Clone, Copy)]
pub struct Token {
    pub kind: TokenKind,
    pub start: u32,
    pub end: u32,
    pub data: u32,
}

impl Token {
    pub fn new(kind: TokenKind, start: u32, end: u32, data: u32) -> Self {
        Self { kind, start, end, data }
    }
}

pub struct Lexer<'a> {
    source: &'a [u8],
    position: u32,
    arena: Option<&'a Arena>,
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a [u8]) -> Self {
        Self {
            source,
            position: 0,
            arena: None,
        }
    }

    pub fn with_arena(source: &'a [u8], arena: &'a Arena) -> Self {
        Self {
            source,
            position: 0,
            arena: Some(arena),
        }
    }

    fn current_byte(&self) -> Option<u8> {
        self.source.get(self.position as usize).copied()
    }

    fn peek_byte(&self, offset: u32) -> Option<u8> {
        self.source.get((self.position + offset) as usize).copied()
    }

    fn advance(&mut self) {
        self.position += 1;
    }

    fn is_alpha(c: u8) -> bool {
        (c >= b'a' && c <= b'z') || (c >= b'A' && c <= b'Z')
    }

    fn is_digit(c: u8) -> bool {
        c >= b'0' && c <= b'9'
    }

    fn is_alnum(c: u8) -> bool {
        Self::is_alpha(c) || Self::is_digit(c) || c == b'_'
    }

    fn skip_whitespace(&mut self) -> Token {
        let start = self.position;
        while let Some(c) = self.current_byte() {
            match c {
                b' ' | b'\t' | b'\r' | b'\n' | b'\v' | b'\f' => {
                    self.advance();
                }
                _ => break,
            }
        }
        Token::new(TokenKind::WhiteSpace, start, self.position, 0)
    }

    fn read_identifier(&mut self) -> Token {
        let start = self.position;
        while let Some(c) = self.current_byte() {
            if Self::is_alnum(c) {
                self.advance();
            } else {
                break;
            }
        }
        Token::new(TokenKind::Identifier, start, self.position, 0)
    }

    fn read_numeric(&mut self) -> Token {
        let start = self.position;
        let mut has_decimal = false;
        let mut has_exponent = false;

        if self.current_byte() == Some(b'0') {
            self.advance();
            match self.current_byte() {
                Some(b'x') | Some(b'X') => {
                    self.advance();
                    while let Some(c) = self.current_byte() {
                        if Self::is_digit(c) || (c >= b'a' && c <= b'f') || (c >= b'A' && c <= b'F') {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    return Token::new(TokenKind::NumericConstant, start, self.position, 0);
                }
                Some(b'.') => {
                    has_decimal = true;
                    self.advance();
                }
                Some(b'8')..=Some(b'7') => {
                    while let Some(c) = self.current_byte() {
                        if c >= b'0' && c <= b'7' {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    if self.current_byte() == Some(b'8') || self.current_byte() == Some(b'9') {
                        while Self::is_digit(self.current_byte().unwrap_or(0)) {
                            self.advance();
                        }
                    }
                    return Token::new(TokenKind::NumericConstant, start, self.position, 0);
                }
                _ => {}
            }
        }

        while let Some(c) = self.current_byte() {
            if Self::is_digit(c) {
                self.advance();
            } else if c == b'.' && !has_decimal && !has_exponent {
                has_decimal = true;
                self.advance();
            } else if (c == b'e' || c == b'E') && !has_exponent {
                has_exponent = true;
                self.advance();
                if self.current_byte() == Some(b'+') || self.current_byte() == Some(b'-') {
                    self.advance();
                }
                while let Some(d) = self.current_byte() {
                    if Self::is_digit(d) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                break;
            } else if c == b'u' || c == b'U' || c == b'l' || c == b'L' || c == b'f' || c == b'F' {
                self.advance();
                break;
            } else {
                break;
            }
        }

        Token::new(TokenKind::NumericConstant, start, self.position, 0)
    }

    fn read_string_literal(&mut self) -> Token {
        let start = self.position;
        let quote = self.current_byte().unwrap_or(b'"');
        self.advance();

        while let Some(c) = self.current_byte() {
            if c == b'\\' {
                self.advance();
                if self.current_byte().is_some() {
                    self.advance();
                }
            } else if c == quote {
                self.advance();
                break;
            } else {
                self.advance();
            }
        }

        Token::new(TokenKind::StringLiteral, start, self.position, 0)
    }

    fn read_line_comment(&mut self) -> Token {
        let start = self.position;
        self.advance();
        self.advance();

        while let Some(c) = self.current_byte() {
            if c == b'\n' {
                break;
            }
            self.advance();
        }

        Token::new(TokenKind::Comment, start, self.position, 0)
    }

    fn read_block_comment(&mut self) -> Token {
        let start = self.position;
        self.advance();
        self.advance();

        while let Some(c) = self.current_byte() {
            if c == b'*' && self.peek_byte(1) == Some(b'/') {
                self.advance();
                self.advance();
                break;
            }
            self.advance();
        }

        Token::new(TokenKind::Comment, start, self.position, 0)
    }

    fn read_preprocessor(&mut self) -> Token {
        let start = self.position;

        while let Some(c) = self.current_byte() {
            if c == b'\n' {
                break;
            }
            self.advance();
        }

        Token::new(TokenKind::Preprocessor, start, self.position, 0)
    }

    fn read_punctuator(&mut self) -> Token {
        let start = self.position;
        let c = self.current_byte().unwrap_or(b' ');

        match c {
            b'[' | b']' | b'(' | b')' | b'{' | b'}' | b'.' | b'->' | b'++' | b'--'
            | b'&' | b'*' | b'+' | b'-' | b'~' | b'!' | b'/' | b'%' | b'<<' | b'>>'
            | b'<' | b'>' | b'<=' | b'>=' | b'==' | b'!=' | b'^' | b'|' | b'&&' | b'||'
            | b'?' | b':' | b';' | b'=' | b'*=' | b'/=' | b'%=' | b'+=' | b'-='
            | b'<<=' | b'>>=' | b'&=' | b'^=' | b'|=' | b',' | b'#' | b'##'
            | b':' => {
                self.advance();
                let next = self.current_byte().unwrap_or(b' ');
                match (c, next) {
                    (b'.', b'.') => {
                        if self.peek_byte(2) == Some(b'.') {
                            self.advance();
                            self.advance();
                        }
                    }
                    (b'<', b'<') => {
                        if self.peek_byte(2) == b'=' {
                            self.advance();
                            self.advance();
                        }
                    }
                    (b'>', b'>') => {
                        if self.peek_byte(2) == b'=' {
                            self.advance();
                            self.advance();
                        }
                    }
                    (b'+', b'+') | (b'-', b'-') | (b'&', b'&') | (b'|', b'|')
                    | (b'<', b'=') | (b'>', b'=') | (b'=', b'=') | (b'!', b'=')
                    | (b'*', b'=') | (b'/', b'=') | (b'%', b'=') | (b'+', b'=')
                    | (b'-', b'=') | (b'&', b'=') | (b'^', b'=') | (b'|', b'=') => {
                        self.advance();
                    }
                    (b'#', b'#') => {
                        self.advance();
                    }
                    _ => {}
                }
            }
            _ => {
                self.advance();
            }
        }

        Token::new(TokenKind::Punctuator, start, self.position, 0)
    }

    pub fn next_token(&mut self) -> Token {
        loop {
            let token = self.scan_token();
            if token.kind == TokenKind::WhiteSpace {
                continue;
            }
            return token;
        }
    }

    fn scan_token(&mut self) -> Token {
        match self.current_byte() {
            None => Token::new(TokenKind::EndOfFile, self.position, self.position, 0),
            Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n') | Some(b'\v') | Some(b'\f') => {
                self.skip_whitespace()
            }
            Some(b'/') => {
                if self.peek_byte(1) == Some(b'/') {
                    self.read_line_comment()
                } else if self.peek_byte(1) == Some(b'*') {
                    self.read_block_comment()
                } else {
                    self.read_punctuator()
                }
            }
            Some(b'#') => self.read_preprocessor(),
            Some(b'"') | Some(b'\'') => self.read_string_literal(),
            Some(c) if Self::is_alpha(c) || c == b'_' => self.read_identifier(),
            Some(c) if Self::is_digit(c) => self.read_numeric(),
            _ => self.read_punctuator(),
        }
    }

    pub fn token_text(&self, token: &Token) -> &'a [u8] {
        &self.source[token.start as usize..token.end as usize]
    }

    pub fn is_keyword(&self, token: &Token) -> bool {
        if token.kind != TokenKind::Identifier {
            return false;
        }
        let text = self.token_text(token);
        matches!(
            text,
            b"auto"
                | b"break"
                | b"case"
                | b"char"
                | b"const"
                | b"continue"
                | b"default"
                | b"do"
                | b"double"
                | b"else"
                | b"enum"
                | b"extern"
                | b"float"
                | b"for"
                | b"goto"
                | b"if"
                | b"inline"
                | b"int"
                | b"long"
                | b"register"
                | b"restrict"
                | b"return"
                | b"short"
                | b"signed"
                | b"sizeof"
                | b"static"
                | b"struct"
                | b"switch"
                | b"typedef"
                | b"union"
                | b"unsigned"
                | b"void"
                | b"volatile"
                | b"while"
                | b"_Bool"
                | b"_Complex"
                | b"_Imaginary"
        )
    }
}

const KEYWORDS: &[&[u8]] = &[
    b"auto", b"break", b"case", b"char", b"const", b"continue", b"default", b"do", b"double",
    b"else", b"enum", b"extern", b"float", b"for", b"goto", b"if", b"inline", b"int", b"long",
    b"register", b"restrict", b"return", b"short", b"signed", b"sizeof", b"static", b"struct",
    b"switch", b"typedef", b"union", b"unsigned", b"void", b"volatile", b"while", b"_Bool",
    b"_Complex", b"_Imaginary",
];

pub fn get_keyword_index(text: &[u8]) -> Option<u16> {
    KEYWORDS
        .iter()
        .position(|&kw| kw == text)
        .map(|i| i as u16 + 256)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lexer_identifiers() {
        let source = b"int main()";
        let mut lexer = Lexer::new(source);
        let token = lexer.next_token();
        assert_eq!(token.kind, TokenKind::Identifier);
        assert_eq!(lexer.token_text(&token), b"int");
    }

    #[test]
    fn test_lexer_numeric() {
        let source = b"42 3.14 0x1A";
        let mut lexer = Lexer::new(source);
        let t1 = lexer.next_token();
        assert_eq!(t1.kind, TokenKind::NumericConstant);
        assert_eq!(lexer.token_text(&t1), b"42");
    }

    #[test]
    fn test_lexer_string() {
        let source = b"\"hello\"";
        let mut lexer = Lexer::new(source);
        let token = lexer.next_token();
        assert_eq!(token.kind, TokenKind::StringLiteral);
        assert_eq!(lexer.token_text(&token), b"\"hello\"");
    }

    #[test]
    fn test_lexer_keywords() {
        let source = b"if while for return";
        let mut lexer = Lexer::new(source);
        assert!(lexer.is_keyword(&lexer.next_token()));
        assert!(lexer.is_keyword(&lexer.next_token()));
        assert!(lexer.is_keyword(&lexer.next_token()));
        assert!(lexer.is_keyword(&lexer.next_token()));
    }

    #[test]
    fn test_lexer_comments() {
        let source = b"// comment\n42".as_slice();
        let mut lexer = Lexer::new(source);
        let t = lexer.next_token();
        assert_eq!(t.kind, TokenKind::Comment);
        let num = lexer.next_token();
        assert_eq!(num.kind, TokenKind::NumericConstant);
    }

    #[test]
    fn test_lexer_preprocessor() {
        let source = b"#include <stdio.h>";
        let mut lexer = Lexer::new(source);
        let token = lexer.next_token();
        assert_eq!(token.kind, TokenKind::Preprocessor);
        assert_eq!(lexer.token_text(&token), b"#include <stdio.h>");
    }
}