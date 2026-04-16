use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset};
use std::io;

pub struct Parser {
    arena: Arena,
    tokens: Vec<Token>,
    current: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Identifier,
    IntConstant,
    CharConstant,
    StringLiteral,
    Punctuator,
    EOF,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
    pub column: u32,
}

impl Parser {
    pub fn new(arena: Arena) -> Self {
        Parser {
            arena,
            tokens: Vec::new(),
            current: 0,
        }
    }

    pub fn parse(&mut self, source: &str) -> Result<NodeOffset, ParseError> {
        self.tokens = self.lex(source);
        self.current = 0;
        self.parse_translation_unit()
    }

    fn lex(&self, source: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut chars = source.chars().peekable();
        let mut line = 1u32;
        let mut column = 1u32;

        while let Some(&ch) = chars.peek() {
            match ch {
                ' ' | '\t' | '\r' => {
                    chars.next();
                    column += 1;
                }
                '\n' => {
                    chars.next();
                    line += 1;
                    column = 1;
                }
                '/' if chars.clone().nth(1) == Some('/') => {
                    while let Some(&c) = chars.peek() {
                        if c == '\n' {
                            break;
                        }
                        chars.next();
                    }
                }
                '/' if chars.clone().nth(1) == Some('*') => {
                    chars.next();
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '*' && chars.peek() == Some(&'/') {
                            chars.next();
                            break;
                        }
                    }
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    let start_column = column;
                    let mut text = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_alphanumeric() || c == '_' {
                            text.push(c);
                            chars.next();
                            column += 1;
                        } else {
                            break;
                        }
                    }
                    let kind = if Self::is_keyword(&text) {
                        TokenKind::Keyword
                    } else {
                        TokenKind::Identifier
                    };
                    tokens.push(Token {
                        kind,
                        text,
                        line,
                        column: start_column,
                    });
                }
                '0'..='9' => {
                    let start_column = column;
                    let mut text = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_alphanumeric() || c == '.' {
                            text.push(c);
                            chars.next();
                            column += 1;
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token {
                        kind: TokenKind::IntConstant,
                        text,
                        line,
                        column: start_column,
                    });
                }
                '\'' => {
                    let start_column = column;
                    chars.next();
                    column += 1;
                    let mut ch = String::new();
                    if let Some(&c) = chars.peek() {
                        if c == '\\' {
                            ch.push(chars.next().unwrap());
                            column += 1;
                            if let Some(&nc) = chars.peek() {
                                ch.push(nc);
                                chars.next();
                                column += 1;
                            }
                        } else {
                            ch.push(c);
                            chars.next();
                            column += 1;
                        }
                    }
                    if chars.peek() == Some(&'\'') {
                        chars.next();
                        column += 1;
                    }
                    tokens.push(Token {
                        kind: TokenKind::CharConstant,
                        text: ch,
                        line,
                        column: start_column,
                    });
                }
                '"' => {
                    let start_column = column;
                    chars.next();
                    column += 1;
                    let mut text = String::new();
                    while let Some(&c) = chars.peek() {
                        if c == '"' {
                            chars.next();
                            column += 1;
                            break;
                        }
                        if c == '\\' {
                            text.push(chars.next().unwrap());
                            column += 1;
                            if let Some(&nc) = chars.peek() {
                                text.push(nc);
                                chars.next();
                                column += 1;
                            }
                            continue;
                        }
                        text.push(c);
                        chars.next();
                        column += 1;
                    }
                    tokens.push(Token {
                        kind: TokenKind::StringLiteral,
                        text,
                        line,
                        column: start_column,
                    });
                }
                _ => {
                    let start_column = column;
                    let mut text = String::new();
                    text.push(chars.next().unwrap());
                    column += 1;

                    if let Some(&next) = chars.peek() {
                        let two_char = format!("{}{}", text, next);
                        if Self::is_punctuator(&two_char) {
                            text.push(chars.next().unwrap());
                            column += 1;
                        }
                    }

                    if !Self::is_punctuator(&text) {
                        if text.chars().all(|c| c.is_whitespace()) {
                            continue;
                        }
                    }

                    tokens.push(Token {
                        kind: TokenKind::Punctuator,
                        text,
                        line,
                        column: start_column,
                    });
                }
            }
        }

        tokens.push(Token {
            kind: TokenKind::EOF,
            text: String::new(),
            line,
            column,
        });

        tokens
    }

    fn is_keyword(text: &str) -> bool {
        matches!(
            text,
            "auto"
                | "break"
                | "case"
                | "char"
                | "const"
                | "continue"
                | "default"
                | "do"
                | "double"
                | "else"
                | "enum"
                | "extern"
                | "float"
                | "for"
                | "goto"
                | "if"
                | "inline"
                | "int"
                | "long"
                | "register"
                | "restrict"
                | "return"
                | "short"
                | "signed"
                | "sizeof"
                | "static"
                | "struct"
                | "switch"
                | "typedef"
                | "union"
                | "unsigned"
                | "void"
                | "volatile"
                | "while"
                | "_Bool"
                | "_Complex"
                | "_Imaginary"
        )
    }

    fn is_punctuator(text: &str) -> bool {
        matches!(
            text,
            "..."
                | ">>="
                | "<<="
                | "+="
                | "-="
                | "*="
                | "/="
                | "%="
                | "&="
                | "^="
                | "|="
                | ">>"
                | "<<"
                | "++"
                | "--"
                | "->"
                | "&&"
                | "||"
                | "<="
                | ">="
                | "=="
                | "!="
                | ";"
                | "{"
                | "}"
                | ","
                | ":"
                | "="
                | "("
                | ")"
                | "["
                | "]"
                | "."
                | "&"
                | "*"
                | "+"
                | "-"
                | "~"
                | "!"
                | "/"
                | "%"
                | "<"
                | ">"
                | "^"
                | "|"
                | "?"
                | "#"
        )
    }

    fn alloc_node(
        &mut self,
        kind: u16,
        data: u32,
        parent: NodeOffset,
        first_child: NodeOffset,
        next_sibling: NodeOffset,
    ) -> NodeOffset {
        let node = CAstNode {
            kind,
            flags: NodeFlags::IS_VALID,
            parent,
            first_child,
            next_sibling,
            data,
        };
        self.arena.alloc(node).unwrap_or(NodeOffset::NULL)
    }

    fn current_token(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn peek_token(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.current + offset)
    }

    fn advance(&mut self) {
        if self.current < self.tokens.len() {
            self.current += 1;
        }
    }

    fn expect(&mut self, text: &str) -> Result<(), ParseError> {
        let token = self.current_token();
        if token.text == text {
            self.advance();
            Ok(())
        } else {
            Err(ParseError {
                message: format!("Expected '{}' but found '{}'", text, token.text),
                line: token.line,
                column: token.column,
            })
        }
    }

    fn skip_punctuator(&mut self, text: &str) -> bool {
        if self.current_token().kind == TokenKind::Punctuator && self.current_token().text == text {
            self.advance();
            true
        } else {
            false
        }
    }

    fn is_type_specifier(&self) -> bool {
        let token = self.current_token();
        if token.kind != TokenKind::Keyword {
            return false;
        }
        matches!(
            token.text.as_str(),
            "void" | "char" | "int" | "float" | "double" | "short" | "long" | "signed" | "unsigned"
                | "struct" | "union" | "enum" | "_Bool" | "_Complex" | "_Imaginary"
        )
    }

    fn is_function_specifier(&self) -> bool {
        let token = self.current_token();
        token.kind == TokenKind::Keyword
            && matches!(token.text.as_str(), "inline" | "_Noreturn" | "noreturn")
    }

    fn is_storage_class_specifier(&self) -> bool {
        let token = self.current_token();
        token.kind == TokenKind::Keyword
            && matches!(
                token.text.as_str(),
                "typedef" | "extern" | "static" | "auto" | "register"
            )
    }

    fn parse_translation_unit(&mut self) -> Result<NodeOffset, ParseError> {
        let mut first_decl = NodeOffset::NULL;
        let mut last_decl = NodeOffset::NULL;

        while self.current_token().kind != TokenKind::EOF {
            match self.parse_external_declaration() {
                Ok(decl) => {
                    if first_decl == NodeOffset::NULL {
                        first_decl = decl;
                        last_decl = decl;
                    } else {
                        let sibling = self.alloc_node(
                            100,
                            0,
                            NodeOffset::NULL,
                            NodeOffset::NULL,
                            NodeOffset::NULL,
                        );
                        if let Some(last) = self.arena.get_mut(last_decl) {
                            last.next_sibling = sibling;
                        }
                        last_decl = sibling;
                    }
                }
                Err(_e) => {
                    self.advance();
                }
            }
        }

        Ok(first_decl)
    }

    fn parse_external_declaration(&mut self) -> Result<NodeOffset, ParseError> {
        let mut specifiers = self.parse_declaration_specifiers()?;
        let mut first_child = specifiers;
        let mut last_child = specifiers;

        while self.current_token().kind == TokenKind::Punctuator
            && self.current_token().text == ","
        {
            self.advance();
            let declarator = self.parse_declarator()?;
            let decl_node = self.alloc_node(20, 0, NodeOffset::NULL, first_child, NodeOffset::NULL);
            if let Some(child) = self.arena.get_mut(first_child) {
                child.parent = decl_node;
            }
            self.link_siblings(&mut first_child, &mut last_child, declarator);
            if self.skip_punctuator(";") {
                break;
            }
        }

        if self.skip_punctuator(";") {
            return Ok(self.alloc_node(20, 0, NodeOffset::NULL, first_child, NodeOffset::NULL));
        }

        if self.skip_punctuator("{") {
            let compound = self.parse_compound_statement(None)?;
            return Ok(self.alloc_node(23, 0, NodeOffset::NULL, first_child, compound));
        }

        Ok(self.alloc_node(20, 0, NodeOffset::NULL, first_child, NodeOffset::NULL))
    }

    fn parse_declaration_specifiers(&mut self) -> Result<NodeOffset, ParseError> {
        let mut first_spec = NodeOffset::NULL;
        let mut last_spec = NodeOffset::NULL;
        let mut has_type_specifier = false;

        while self.current_token().kind == TokenKind::Keyword
            || self.is_storage_class_specifier()
            || self.is_type_qualifier()
        {
            if self.is_storage_class_specifier() {
                let spec = self.parse_storage_class_specifier()?;
                self.link_siblings(&mut first_spec, &mut last_spec, spec);
            } else if self.is_type_specifier() {
                has_type_specifier = true;
                let spec = self.parse_type_specifier()?;
                self.link_siblings(&mut first_spec, &mut last_spec, spec);
            } else if self.is_type_qualifier() {
                let spec = self.parse_type_qualifier()?;
                self.link_siblings(&mut first_spec, &mut last_spec, spec);
            } else if self.is_function_specifier() {
                let spec = self.parse_function_specifier()?;
                self.link_siblings(&mut first_spec, &mut last_spec, spec);
            } else {
                break;
            }
        }

        if first_spec == NodeOffset::NULL && !has_type_specifier {
            return Err(ParseError {
                message: "Expected type specifier".to_string(),
                line: self.current_token().line,
                column: self.current_token().column,
            });
        }

        Ok(first_spec)
    }

    fn is_type_qualifier(&self) -> bool {
        let token = self.current_token();
        token.kind == TokenKind::Keyword
            && matches!(token.text.as_str(), "const" | "restrict" | "volatile")
    }

    fn parse_storage_class_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        let token = self.current_token();
        let text = token.text.clone();
        self.advance();

        let kind = match text.as_str() {
            "typedef" => 101,
            "extern" => 102,
            "static" => 103,
            "auto" => 104,
            "register" => 105,
            _ => 101,
        };

        Ok(self.alloc_node(kind, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
    }

    fn parse_type_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        let token = self.current_token();
        let text = token.text.clone();
        self.advance();

        let kind = match text.as_str() {
            "void" => 1,
            "int" => 2,
            "char" => 3,
            "float" => 83,
            "double" => 84,
            "short" => 10,
            "long" => 11,
            "signed" => 12,
            "unsigned" => 13,
            "struct" => return self.parse_struct_specifier(),
            "union" => return self.parse_union_specifier(),
            "enum" => return self.parse_enum_specifier(),
            "_Bool" => 14,
            "_Complex" => 15,
            _ => 2,
        };

        Ok(self.alloc_node(kind, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
    }

    fn parse_struct_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("{")?;
        let mut first_member = NodeOffset::NULL;
        let mut last_member = NodeOffset::NULL;

        while !self.skip_punctuator("}") {
            let member_decl = self.parse_struct_declaration()?;
            self.link_siblings(&mut first_member, &mut last_member, member_decl);
        }

        Ok(self.alloc_node(4, 0, NodeOffset::NULL, first_member, NodeOffset::NULL))
    }

    fn parse_union_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("{")?;
        let mut first_member = NodeOffset::NULL;
        let mut last_member = NodeOffset::NULL;

        while !self.skip_punctuator("}") {
            let member_decl = self.parse_struct_declaration()?;
            self.link_siblings(&mut first_member, &mut last_member, member_decl);
        }

        Ok(self.alloc_node(5, 0, NodeOffset::NULL, first_member, NodeOffset::NULL))
    }

    fn parse_struct_declaration(&mut self) -> Result<NodeOffset, ParseError> {
        let specifiers = self.parse_declaration_specifiers()?;
        let mut first_declarator = NodeOffset::NULL;
        let mut last_declarator = NodeOffset::NULL;

        while !self.skip_punctuator(";") {
            if self.current_token().kind == TokenKind::Punctuator
                && self.current_token().text == ","
            {
                self.advance();
                continue;
            }
            let declarator = self.parse_declarator()?;
            self.link_siblings(&mut first_declarator, &mut last_declarator, declarator);
        }

        Ok(self.alloc_node(
            25,
            0,
            NodeOffset::NULL,
            specifiers,
            first_declarator,
        ))
    }

    fn parse_enum_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        let mut first_const = NodeOffset::NULL;
        let mut last_const = NodeOffset::NULL;

        if self.skip_punctuator("{") {
            while !self.skip_punctuator("}") {
                if self.current_token().kind != TokenKind::Identifier {
                    break;
                }
                let name = self.current_token().text.clone();
                self.advance();

                let value = if self.skip_punctuator("=") {
                    let const_expr = self.parse_constant_expression()?;
                    const_expr.0
                } else {
                    0
                };

                let const_node =
                    self.alloc_node(26, value, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL);
                let ident_node = self.alloc_node(
                    60,
                    0,
                    const_node,
                    NodeOffset::NULL,
                    NodeOffset::NULL,
                );
                if let Some(cn) = self.arena.get_mut(const_node) {
                    cn.first_child = ident_node;
                }
                self.link_siblings(&mut first_const, &mut last_const, const_node);

                if self.skip_punctuator("}") {
                    break;
                }
                self.skip_punctuator(",");
            }
        }

        Ok(self.alloc_node(6, 0, NodeOffset::NULL, first_const, NodeOffset::NULL))
    }

    fn parse_type_qualifier(&mut self) -> Result<NodeOffset, ParseError> {
        let token = self.current_token();
        let text = token.text.clone();
        self.advance();

        let kind = match text.as_str() {
            "const" => 90,
            "restrict" => 91,
            "volatile" => 92,
            _ => 90,
        };

        Ok(self.alloc_node(kind, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
    }

    fn parse_function_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        let token = self.current_token();
        let text = token.text.clone();
        self.advance();

        let kind = match text.as_str() {
            "inline" => 93,
            "_Noreturn" | "noreturn" => 94,
            _ => 93,
        };

        Ok(self.alloc_node(kind, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
    }

    fn parse_declarator(&mut self) -> Result<NodeOffset, ParseError> {
        let mut pointer_node = NodeOffset::NULL;
        let mut last_pointer = NodeOffset::NULL;

        while self.skip_punctuator("*") {
            let ptr = self.alloc_node(7, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL);
            if pointer_node == NodeOffset::NULL {
                pointer_node = ptr;
                last_pointer = ptr;
            } else if let Some(lp) = self.arena.get_mut(last_pointer) {
                lp.next_sibling = ptr;
                last_pointer = ptr;
            }
        }

        let direct_decl = self.parse_direct_declarator()?;

        if pointer_node != NodeOffset::NULL {
            if let Some(pp) = self.arena.get_mut(pointer_node) {
                pp.first_child = direct_decl;
            }
            return Ok(pointer_node);
        }

        Ok(direct_decl)
    }

    fn parse_direct_declarator(&mut self) -> Result<NodeOffset, ParseError> {
        let mut declarator = NodeOffset::NULL;

        if self.current_token().kind == TokenKind::Identifier {
            let name = self.current_token().text.clone();
            self.advance();
            declarator = self.alloc_node(60, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL);
        } else if self.skip_punctuator("(") {
            let inner = self.parse_declarator()?;
            self.expect(")")?;
            declarator = inner;
        }

        loop {
            if self.skip_punctuator("[") {
                let size = if self.current_token().text != "]" {
                    self.parse_constant_expression()?.0
                } else {
                    0
                };
                self.expect("]")?;
                let arr = self.alloc_node(8, size, NodeOffset::NULL, declarator, NodeOffset::NULL);
                declarator = arr;
            } else if self.skip_punctuator("(") {
                if self.current_token().text == ")" {
                    self.advance();
                    declarator = self.alloc_node(9, 0, NodeOffset::NULL, declarator, NodeOffset::NULL);
                } else {
                    let params = self.parse_parameter_list()?;
                    self.expect(")")?;
                    declarator = self.alloc_node(9, 0, NodeOffset::NULL, declarator, params);
                }
            } else {
                break;
            }
        }

        Ok(declarator)
    }

    fn parse_parameter_list(&mut self) -> Result<NodeOffset, ParseError> {
        let mut first_param = NodeOffset::NULL;
        let mut last_param = NodeOffset::NULL;

        if self.current_token().text == ")" {
            return Ok(first_param);
        }

        loop {
            if self.current_token().text == ")" {
                break;
            }
            let param = self.parse_parameter_declaration()?;
            self.link_siblings(&mut first_param, &mut last_param, param);

            if self.skip_punctuator(")") {
                break;
            }
            if !self.skip_punctuator(",") {
                break;
            }
            if self.current_token().text == "..." {
                self.advance();
                break;
            }
        }

        Ok(first_param)
    }

    fn parse_parameter_declaration(&mut self) -> Result<NodeOffset, ParseError> {
        if self.is_type_specifier() {
            let specifiers = self.parse_declaration_specifiers()?;
            let declarator = if self.is_declarator_start() {
                self.parse_declarator()?
            } else {
                NodeOffset::NULL
            };

            Ok(self.alloc_node(
                24,
                0,
                NodeOffset::NULL,
                specifiers,
                declarator,
            ))
        } else {
            Ok(self.alloc_node(
                24,
                0,
                NodeOffset::NULL,
                NodeOffset::NULL,
                NodeOffset::NULL,
            ))
        }
    }

    fn is_declarator_start(&self) -> bool {
        let token = self.current_token();
        token.kind == TokenKind::Identifier
            || (token.kind == TokenKind::Punctuator && token.text == "(")
            || (token.kind == TokenKind::Punctuator && token.text == "*")
    }

    fn parse_compound_statement(&mut self, parent: Option<NodeOffset>) -> Result<NodeOffset, ParseError> {
        let mut first_item = NodeOffset::NULL;
        let mut last_item = NodeOffset::NULL;

        while !self.skip_punctuator("}") {
            if self.skip_punctuator(";") {
                let empty = self.alloc_node(48, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL);
                self.link_siblings(&mut first_item, &mut last_item, empty);
                continue;
            }

            if self.is_type_specifier() || self.is_storage_class_specifier() {
                let decl = self.parse_declaration()?;
                self.link_siblings(&mut first_item, &mut last_item, decl);
            } else {
                let stmt = self.parse_statement()?;
                self.link_siblings(&mut first_item, &mut last_item, stmt);
            }
        }

        Ok(self.alloc_node(40, 0, NodeOffset::NULL, first_item, NodeOffset::NULL))
    }

    fn parse_declaration(&mut self) -> Result<NodeOffset, ParseError> {
        let specifiers = self.parse_declaration_specifiers()?;
        let mut first_init = NodeOffset::NULL;
        let mut last_init = NodeOffset::NULL;

        if !self.skip_punctuator(";") {
            loop {
                let declarator = self.parse_declarator()?;
                let mut init = declarator;

                if self.skip_punctuator("=") {
                    let init_expr = self.parse_initializer()?;
                    init = self.alloc_node(
                        73,
                        19,
                        NodeOffset::NULL,
                        declarator,
                        init_expr,
                    );
                }

                self.link_siblings(&mut first_init, &mut last_init, init);

                if self.skip_punctuator(";") {
                    break;
                }
                if !self.skip_punctuator(",") {
                    self.skip_punctuator(";");
                    break;
                }
            }
        }

        Ok(self.alloc_node(
            21,
            0,
            NodeOffset::NULL,
            specifiers,
            first_init,
        ))
    }

    fn parse_initializer(&mut self) -> Result<NodeOffset, ParseError> {
        if self.skip_punctuator("{") {
            let mut first_elem = NodeOffset::NULL;
            let mut last_elem = NodeOffset::NULL;

            loop {
                let elem = self.parse_initializer()?;
                self.link_siblings(&mut first_elem, &mut last_elem, elem);

                if self.skip_punctuator("}") {
                    break;
                }
                if !self.skip_punctuator(",") {
                    break;
                }
            }

            return Ok(first_elem);
        }

        self.parse_assignment_expression()
    }

    fn parse_statement(&mut self) -> Result<NodeOffset, ParseError> {
        let token = self.current_token();

        if token.kind == TokenKind::Punctuator && token.text == "{" {
            self.advance();
            return self.parse_compound_statement(None);
        }

        if token.kind == TokenKind::Keyword {
            match token.text.as_str() {
                "if" => return self.parse_if_statement(),
                "while" => return self.parse_while_statement(),
                "for" => return self.parse_for_statement(),
                "return" => return self.parse_return_statement(),
                "break" => {
                    self.advance();
                    self.skip_punctuator(";");
                    return Ok(self.alloc_node(46, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL));
                }
                "continue" => {
                    self.advance();
                    self.skip_punctuator(";");
                    return Ok(self.alloc_node(47, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL));
                }
                "switch" => return self.parse_switch_statement(),
                "goto" => return self.parse_goto_statement(),
                "do" => return self.parse_do_statement(),
                _ => {}
            }
        }

        self.parse_expression_statement()
    }

    fn parse_if_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("if")?;
        self.expect("(")?;
        let condition = self.parse_expression()?;
        self.expect(")")?;
        let then_stmt = self.parse_statement()?;
        let else_stmt = if self.skip_punctuator("else") {
            Some(self.parse_statement()?)
        } else {
            None
        };

        let else_node = self.alloc_node(0, 0, NodeOffset::NULL, then_stmt, else_stmt.unwrap_or(NodeOffset::NULL));
        Ok(self.alloc_node(41, 0, NodeOffset::NULL, condition, else_node))
    }

    fn parse_while_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("while")?;
        self.expect("(")?;
        let condition = self.parse_expression()?;
        self.expect(")")?;
        let body = self.parse_statement()?;

        Ok(self.alloc_node(
            42,
            0,
            NodeOffset::NULL,
            condition,
            body,
        ))
    }

    fn parse_for_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("for")?;
        self.expect("(")?;

        let init = if self.is_type_specifier() {
            self.parse_declaration()?
        } else if !self.skip_punctuator(";") {
            let expr = self.parse_expression()?;
            self.skip_punctuator(";");
            expr
        } else {
            self.alloc_node(48, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL)
        };

        let condition = if !self.skip_punctuator(";") {
            let expr = self.parse_expression()?;
            self.skip_punctuator(";");
            expr
        } else {
            self.alloc_node(61, 1, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL)
        };

        let increment = if !self.skip_punctuator(")") {
            let expr = self.parse_expression()?;
            self.skip_punctuator(")");
            expr
        } else {
            NodeOffset::NULL
        };

        let body = self.parse_statement()?;

        let init_node = self.alloc_node(0, 0, NodeOffset::NULL, init, condition);
        let increment_node = self.alloc_node(0, 0, NodeOffset::NULL, increment, body);
        Ok(self.alloc_node(43, 0, NodeOffset::NULL, init_node, increment_node))
    }

    fn parse_do_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("do")?;
        let body = self.parse_statement()?;
        self.expect("while")?;
        self.expect("(")?;
        let condition = self.parse_expression()?;
        self.expect(")")?;
        self.skip_punctuator(";");

        Ok(self.alloc_node(
            42,
            1,
            NodeOffset::NULL,
            body,
            condition,
        ))
    }

    fn parse_switch_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("switch")?;
        self.expect("(")?;
        let condition = self.parse_expression()?;
        self.expect(")")?;
        let body = self.parse_statement()?;

        Ok(self.alloc_node(
            50,
            0,
            NodeOffset::NULL,
            condition,
            body,
        ))
    }

    fn parse_goto_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("goto")?;
        if self.current_token().kind == TokenKind::Identifier {
            let label = self.current_token().text.clone();
            self.advance();
            self.skip_punctuator(";");
            Ok(self.alloc_node(49, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
        } else {
            self.skip_punctuator(";");
            Ok(self.alloc_node(49, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
        }
    }

    fn parse_return_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("return")?;
        let expr = if !self.skip_punctuator(";") {
            let e = self.parse_expression()?;
            self.skip_punctuator(";");
            e
        } else {
            NodeOffset::NULL
        };

        Ok(self.alloc_node(44, 0, NodeOffset::NULL, expr, NodeOffset::NULL))
    }

    fn parse_expression_statement(&mut self) -> Result<NodeOffset, ParseError> {
        let expr = if !self.skip_punctuator(";") {
            let e = self.parse_expression()?;
            self.skip_punctuator(";");
            e
        } else {
            NodeOffset::NULL
        };

        Ok(self.alloc_node(45, 0, NodeOffset::NULL, expr, NodeOffset::NULL))
    }

    fn parse_expression(&mut self) -> Result<NodeOffset, ParseError> {
        self.parse_comma_expression()
    }

    fn parse_comma_expression(&mut self) -> Result<NodeOffset, ParseError> {
        let mut left = self.parse_assignment_expression()?;

        if self.skip_punctuator(",") {
            let right = self.parse_comma_expression()?;
            left = self.alloc_node(72, 0, NodeOffset::NULL, left, right);
        }

        Ok(left)
    }

    fn parse_assignment_expression(&mut self) -> Result<NodeOffset, ParseError> {
        let left = self.parse_conditional_expression()?;

        let op = if self.current_token().kind == TokenKind::Punctuator {
            match self.current_token().text.as_str() {
                "=" => Some((19, false)),
                "+=" => Some((1, true)),
                "-=" => Some((2, true)),
                "*=" => Some((3, true)),
                "/=" => Some((4, true)),
                "%=" => Some((5, true)),
                "&=" => Some((14, true)),
                "^=" => Some((16, true)),
                "|=" => Some((15, true)),
                ">>=" => Some((18, true)),
                "<<=" => Some((17, true)),
                _ => None,
            }
        } else {
            None
        };

        if let Some((op_code, compound)) = op {
            self.advance();
            let right = self.parse_assignment_expression()?;
            return Ok(self.alloc_node(73, op_code, NodeOffset::NULL, left, right));
        }

        Ok(left)
    }

    fn parse_conditional_expression(&mut self) -> Result<NodeOffset, ParseError> {
        let condition = self.parse_binary_expression()?;

        if self.skip_punctuator("?") {
            let then_expr = self.parse_expression()?;
            self.expect(":")?;
            let else_expr = self.parse_conditional_expression()?;
            let else_node = self.alloc_node(0, 0, NodeOffset::NULL, then_expr, else_expr);
            return Ok(self.alloc_node(66, 0, NodeOffset::NULL, condition, else_node));
        }

        Ok(condition)
    }

    fn parse_binary_expression(&mut self) -> Result<NodeOffset, ParseError> {
        self.parse_binary_op(0)
    }

    fn parse_binary_op(&mut self, precedence: u8) -> Result<NodeOffset, ParseError> {
        let mut left = if precedence + 1 < 15 {
            self.parse_binary_op(precedence + 1)?
        } else {
            self.parse_unary_expression()?
        };

        loop {
            let (op_prec, op_code) = self.get_binary_operator();
            if op_prec < precedence {
                break;
            }

            self.advance();
            let right = if precedence + 1 < 15 {
                self.parse_binary_op(precedence + 1)?
            } else {
                self.parse_unary_expression()?
            };

            left = self.alloc_node(64, op_code, NodeOffset::NULL, left, right);
        }

        Ok(left)
    }

    fn get_binary_operator(&self) -> (u8, u32) {
        let token = self.current_token();
        if token.kind != TokenKind::Punctuator {
            return (0, 0);
        }

        match token.text.as_str() {
            "||" => (1, 13),
            "&&" => (2, 12),
            "|" => (3, 15),
            "^" => (4, 16),
            "&" => (5, 14),
            "==" | "!=" => (6, 0),
            "<" | ">" | "<=" | ">=" => (7, 0),
            "<<" | ">>" => (8, 0),
            "+" | "-" => (9, 0),
            "*" | "/" | "%" => (10, 0),
            _ => (0, 0),
        }
    }

    fn parse_unary_expression(&mut self) -> Result<NodeOffset, ParseError> {
        let token = self.current_token();

        if token.kind == TokenKind::Punctuator {
            match token.text.as_str() {
                "++" => {
                    self.advance();
                    let operand = self.parse_unary_expression()?;
                    return Ok(self.alloc_node(65, 6, NodeOffset::NULL, operand, NodeOffset::NULL));
                }
                "--" => {
                    self.advance();
                    let operand = self.parse_unary_expression()?;
                    return Ok(self.alloc_node(65, 7, NodeOffset::NULL, operand, NodeOffset::NULL));
                }
                "+" => {
                    self.advance();
                    let operand = self.parse_cast_expression()?;
                    return Ok(self.alloc_node(65, 0, NodeOffset::NULL, operand, NodeOffset::NULL));
                }
                "-" => {
                    self.advance();
                    let operand = self.parse_cast_expression()?;
                    return Ok(self.alloc_node(65, 1, NodeOffset::NULL, operand, NodeOffset::NULL));
                }
                "!" => {
                    self.advance();
                    let operand = self.parse_cast_expression()?;
                    return Ok(self.alloc_node(65, 2, NodeOffset::NULL, operand, NodeOffset::NULL));
                }
                "~" => {
                    self.advance();
                    let operand = self.parse_cast_expression()?;
                    return Ok(self.alloc_node(65, 3, NodeOffset::NULL, operand, NodeOffset::NULL));
                }
                "&" => {
                    self.advance();
                    let operand = self.parse_cast_expression()?;
                    return Ok(self.alloc_node(65, 4, NodeOffset::NULL, operand, NodeOffset::NULL));
                }
                "*" => {
                    self.advance();
                    let operand = self.parse_cast_expression()?;
                    return Ok(self.alloc_node(65, 5, NodeOffset::NULL, operand, NodeOffset::NULL));
                }
                "sizeof" => {
                    return self.parse_sizeof_expression();
                }
                _ => {}
            }
        }

        self.parse_postfix_expression()
    }

    fn parse_sizeof_expression(&mut self) -> Result<NodeOffset, ParseError> {
        self.advance();

        let operand = if self.skip_punctuator("(") {
            if self.is_type_specifier() {
                let specifiers = self.parse_declaration_specifiers()?;
                self.expect(")")?;
                self.alloc_node(71, 0, NodeOffset::NULL, specifiers, NodeOffset::NULL)
            } else {
                let expr = self.parse_expression()?;
                self.expect(")")?;
                self.alloc_node(71, 1, NodeOffset::NULL, expr, NodeOffset::NULL)
            }
        } else {
            self.parse_unary_expression()?
        };

        Ok(self.alloc_node(71, 0, NodeOffset::NULL, operand, NodeOffset::NULL))
    }

    fn parse_cast_expression(&mut self) -> Result<NodeOffset, ParseError> {
        if self.skip_punctuator("(") {
            if self.is_type_specifier() {
                let type_spec = self.parse_declaration_specifiers()?;
                if self.skip_punctuator(")") {
                    let expr = self.parse_cast_expression()?;
                    return Ok(self.alloc_node(70, 0, NodeOffset::NULL, type_spec, expr));
                }
            }

            let expr = self.parse_expression()?;
            self.expect(")")?;
            return Ok(expr);
        }

        self.parse_unary_expression()
    }

    fn parse_postfix_expression(&mut self) -> Result<NodeOffset, ParseError> {
        let mut expr = self.parse_primary_expression()?;

        loop {
            if self.skip_punctuator("[") {
                let index = self.parse_expression()?;
                self.expect("]")?;
                expr = self.alloc_node(68, 0, NodeOffset::NULL, expr, index);
            } else if self.skip_punctuator("(") {
                let args = self.parse_argument_expression_list()?;
                self.expect(")")?;
                expr = self.alloc_node(67, 0, NodeOffset::NULL, expr, args);
            } else if self.skip_punctuator(".") {
                if self.current_token().kind == TokenKind::Identifier {
                    let member = self.current_token().text.clone();
                    self.advance();
                    let member_node = self.alloc_node(60, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL);
                    expr = self.alloc_node(69, 0, NodeOffset::NULL, expr, member_node);
                }
            } else if self.skip_punctuator("->") {
                if self.current_token().kind == TokenKind::Identifier {
                    let member = self.current_token().text.clone();
                    self.advance();
                    let member_node = self.alloc_node(60, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL);
                    expr = self.alloc_node(69, 1, NodeOffset::NULL, expr, member_node);
                }
            } else if self.skip_punctuator("++") {
                expr = self.alloc_node(65, 6, NodeOffset::NULL, expr, NodeOffset::NULL);
            } else if self.skip_punctuator("--") {
                expr = self.alloc_node(65, 7, NodeOffset::NULL, expr, NodeOffset::NULL);
            } else {
                break;
            }
        }

        Ok(expr)
    }

    fn parse_argument_expression_list(&mut self) -> Result<NodeOffset, ParseError> {
        let mut first_arg = NodeOffset::NULL;
        let mut last_arg = NodeOffset::NULL;

        if self.current_token().text != ")" {
            loop {
                let arg = self.parse_assignment_expression()?;
                self.link_siblings(&mut first_arg, &mut last_arg, arg);

                if self.skip_punctuator(")") {
                    break;
                }
                if !self.skip_punctuator(",") {
                    break;
                }
            }
        }

        Ok(first_arg)
    }

    fn parse_primary_expression(&mut self) -> Result<NodeOffset, ParseError> {
        let token = self.current_token();

        match token.kind {
            TokenKind::Identifier => {
                let name = token.text.clone();
                self.advance();
                Ok(self.alloc_node(60, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
            }
            TokenKind::IntConstant => {
                let value = token.text.parse::<u32>().unwrap_or(0);
                self.advance();
                Ok(self.alloc_node(61, value, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
            }
            TokenKind::CharConstant => {
                self.advance();
                Ok(self.alloc_node(62, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
            }
            TokenKind::StringLiteral => {
                self.advance();
                Ok(self.alloc_node(63, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
            }
            TokenKind::Punctuator if token.text == "(" => {
                self.advance();
                let expr = self.parse_expression()?;
                self.expect(")")?;
                Ok(expr)
            }
            _ => Ok(NodeOffset::NULL),
        }
    }

    fn parse_constant_expression(&mut self) -> Result<NodeOffset, ParseError> {
        self.parse_conditional_expression()
    }

    fn link_siblings(&mut self, first: &mut NodeOffset, last: &mut NodeOffset, node: NodeOffset) {
        if node == NodeOffset::NULL {
            return;
        }
        if *first == NodeOffset::NULL {
            *first = node;
            *last = node;
        } else {
            if let Some(l) = self.arena.get_mut(*last) {
                l.next_sibling = node;
            }
            *last = node;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_parse_simple_function() {
        let temp_file = NamedTempFile::new().unwrap();
        let arena = Arena::new(temp_file.path(), 1024).unwrap();
        let mut parser = Parser::new(arena);

        let source = "int main() { return 0; }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_declarations() {
        let temp_file = NamedTempFile::new().unwrap();
        let arena = Arena::new(temp_file.path(), 1024).unwrap();
        let mut parser = Parser::new(arena);

        let source = "int x; char c;";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_expressions() {
        let temp_file = NamedTempFile::new().unwrap();
        let arena = Arena::new(temp_file.path(), 1024).unwrap();
        let mut parser = Parser::new(arena);

        let source = "int x = 5 + 3 * 2;";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }
}
