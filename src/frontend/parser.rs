use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset, SourceLocation};
use crate::frontend::preprocessor;

pub struct Parser {
    pub arena: Arena,
    pub tokens: Vec<Token>,
    pub current: usize,
    /// Track typedef names so they can be recognized as type specifiers.
    pub typedef_names: std::collections::HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Identifier,
    IntConstant,
    FloatConstant,
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
    pub file: String,
}

impl From<preprocessor::Token> for Token {
    fn from(pp_token: preprocessor::Token) -> Self {
        let kind = match pp_token.kind {
            preprocessor::TokenKind::Identifier => TokenKind::Identifier,
            preprocessor::TokenKind::Keyword => TokenKind::Keyword,
            preprocessor::TokenKind::IntLiteral => TokenKind::IntConstant,
            preprocessor::TokenKind::FloatLiteral => TokenKind::FloatConstant,
            preprocessor::TokenKind::CharLiteral => TokenKind::CharConstant,
            preprocessor::TokenKind::StringLiteral => TokenKind::StringLiteral,
            preprocessor::TokenKind::Punctuator => TokenKind::Punctuator,
            preprocessor::TokenKind::Preprocessor => TokenKind::Punctuator,
            preprocessor::TokenKind::Whitespace => TokenKind::Punctuator,
            preprocessor::TokenKind::Comment => TokenKind::Punctuator,
            preprocessor::TokenKind::EndOfFile => TokenKind::EOF,
        };
        Token {
            kind,
            text: pp_token.text,
            line: pp_token.line,
            column: pp_token.column,
            file: pp_token.file,
        }
    }
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
            typedef_names: std::collections::HashSet::new(),
        }
    }

    pub fn parse(&mut self, source: &str) -> Result<NodeOffset, ParseError> {
        self.tokens = self.lex(source);
        self.current = 0;
        self.parse_translation_unit()
    }

    pub fn parse_tokens(
        &mut self,
        tokens: Vec<preprocessor::Token>,
    ) -> Result<NodeOffset, ParseError> {
        self.tokens = tokens
            .into_iter()
            .filter(|t| {
                !matches!(
                    t.kind,
                    preprocessor::TokenKind::Whitespace | preprocessor::TokenKind::Comment
                )
            })
            .map(Token::from)
            .collect();
        self.tokens.push(Token {
            kind: TokenKind::EOF,
            text: String::new(),
            line: 0,
            column: 0,
            file: String::new(),
        });
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
                        file: String::new(),
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
                        file: String::new(),
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
                        file: String::new(),
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
                        file: String::new(),
                    });
                }
                _ => {
                    let start_column = column;
                    let mut text = String::new();
                    text.push(chars.next().unwrap());
                    column += 1;

                    if let Some(&next) = chars.peek() {
                        let two_char = format!("{}{}", text, next);
                        // Check for three-character punctuators first (e.g., ..., >>=, <<=)
                        let mut cloned = chars.clone();
                        cloned.next(); // skip `next`
                        if let Some(&next2) = cloned.peek() {
                            let three_char = format!("{}{}{}", text, next, next2);
                            if Self::is_punctuator(&three_char) {
                                text.push(chars.next().unwrap());
                                column += 1;
                                text.push(chars.next().unwrap());
                                column += 1;
                            } else if Self::is_punctuator(&two_char) {
                                text.push(chars.next().unwrap());
                                column += 1;
                            }
                        } else if Self::is_punctuator(&two_char) {
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
                        file: String::new(),
                    });
                }
            }
        }

        tokens.push(Token {
            kind: TokenKind::EOF,
            text: String::new(),
            line,
            column,
            file: String::new(),
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
                | "_Atomic"
                | "_Noreturn"
                | "typeof"
                | "__typeof__"
                | "__attribute__"
                | "__extension__"
                | "__label__"
                | "__asm__"
                | "__asm"
                | "asm"
                | "__inline"
                | "__inline__"
                | "__volatile__"
                | "__volatile"
                | "__restrict"
                | "__restrict__"
                | "__const"
                | "__const__"
                | "__signed__"
                | "__signed"
                | "__noreturn__"
        ) || text.starts_with("__builtin_")
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

    pub fn alloc_node(
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
            last_child: NodeOffset::NULL,
            next_sibling,
            prev_sibling: NodeOffset::NULL,
            child_count: 0,
            data,
            source: SourceLocation::unknown(),
            payload_offset: NodeOffset::NULL,
            payload_len: 0,
        };
        self.arena.alloc(node).unwrap_or(NodeOffset::NULL)
    }

    pub fn current_token(&self) -> &Token {
        self.tokens
            .get(self.current)
            .unwrap_or_else(|| self.tokens.last().unwrap())
    }

    pub fn is_at_end(&self) -> bool {
        self.current_token().kind == TokenKind::EOF
    }

    pub fn peek_token(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.current + offset)
    }

    pub fn advance(&mut self) {
        if self.current < self.tokens.len() {
            self.current += 1;
        }
    }

    pub fn expect(&mut self, text: &str) -> Result<(), ParseError> {
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

    pub fn skip_punctuator(&mut self, text: &str) -> bool {
        if self.current_token().kind == TokenKind::Punctuator && self.current_token().text == text {
            self.advance();
            true
        } else {
            false
        }
    }

    pub fn is_type_specifier(&self) -> bool {
        let token = self.current_token();
        if token.kind != TokenKind::Keyword && token.kind != TokenKind::Identifier {
            return false;
        }
        if matches!(
            token.text.as_str(),
            "void"
                | "char"
                | "int"
                | "float"
                | "double"
                | "short"
                | "long"
                | "signed"
                | "unsigned"
                | "struct"
                | "union"
                | "enum"
                | "_Bool"
                | "_Complex"
                | "_Imaginary"
                | "typeof"
                | "__typeof__"
                | "__signed__"
                | "__signed"
        ) {
            return true;
        }
        // Check if identifier is a known typedef name
        if token.kind == TokenKind::Identifier && self.typedef_names.contains(&token.text) {
            return true;
        }
        false
    }

    fn is_gnu_signed_keyword(&self) -> bool {
        let token = self.current_token();
        (token.kind == TokenKind::Keyword || token.kind == TokenKind::Identifier)
            && matches!(token.text.as_str(), "__signed__" | "__signed")
    }

    fn is_function_specifier(&self) -> bool {
        let token = self.current_token();
        (token.kind == TokenKind::Keyword || token.kind == TokenKind::Identifier)
            && matches!(
                token.text.as_str(),
                "inline"
                    | "__inline"
                    | "__inline__"
                    | "_Noreturn"
                    | "noreturn"
                    | "__noreturn__"
            )
    }

    pub fn is_storage_class_specifier(&self) -> bool {
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
                        if let Some(last) = self.arena.get_mut(last_decl) {
                            last.next_sibling = decl;
                        }
                        last_decl = decl;
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
        if self.current_token().kind == TokenKind::Keyword
            && self.current_token().text == "__extension__"
        {
            return self.parse_extension_wrapper();
        }

        // Skip __extension__ at the start of external declarations
        if self.current_token().kind == TokenKind::Identifier
            && self.current_token().text == "__extension__"
        {
            self.advance();
        }

        // Check if this is a typedef before parsing specifiers
        let is_typedef = self.current_token().text == "typedef";

        let specifiers = self.parse_declaration_specifiers()?;
        let mut first_child = specifiers;
        // Walk to the end of the specifier chain so link_siblings doesn't orphan
        // intermediate specifier nodes (e.g., `void` in `static void foo()`).
        let mut last_child = specifiers;
        if last_child != NodeOffset::NULL {
            loop {
                let ns = self.arena.get(last_child).map(|n| n.next_sibling).unwrap_or(NodeOffset::NULL);
                if ns == NodeOffset::NULL {
                    break;
                }
                last_child = ns;
            }
        }

        if self.is_declarator_start() {
            let declarator = self.parse_declarator()?;

            // If this is a typedef, register the declared name
            if is_typedef && declarator != NodeOffset::NULL {
                if let Some(name) = self.find_declarator_name(declarator) {
                    self.typedef_names.insert(name);
                }
            }

            // Handle __asm__("...") after declarator (GCC redirect)
            self.skip_asm_label();

            if let Some(attr_result) = self.parse_attribute_after_declarator() {
                let attr = attr_result?;
                // Link post-declarator attributes into the declaration child chain
                self.link_siblings(&mut first_child, &mut last_child, attr);
            }

            // Handle __asm__ after attribute too
            self.skip_asm_label();

            // Check for initializer: `= expr`
            if self.skip_punctuator("=") {
                let init = self.parse_initializer()?;
                // Create an init-declarator (kind=73) wrapping the declarator
                // with the initializer as a sibling of the declarator's first child
                let init_decl = self.alloc_node(73, 0, NodeOffset::NULL, declarator, NodeOffset::NULL);
                // Link the initializer as next_sibling of the declarator
                if let Some(decl_node) = self.arena.get_mut(declarator) {
                    decl_node.next_sibling = init;
                }
                // Create a var_decl (kind=21) with the specifiers and init-declarator
                let var_decl = self.alloc_node(21, 0, NodeOffset::NULL, specifiers, NodeOffset::NULL);
                // Link init_decl as sibling of specifiers
                if specifiers != NodeOffset::NULL {
                    let mut tail = specifiers;
                    loop {
                        let ns = self.arena.get(tail).map(|n| n.next_sibling).unwrap_or(NodeOffset::NULL);
                        if ns == NodeOffset::NULL { break; }
                        tail = ns;
                    }
                    if let Some(tail_node) = self.arena.get_mut(tail) {
                        tail_node.next_sibling = init_decl;
                    }
                }
                // Wrap in declaration (kind=20)
                first_child = var_decl;
                last_child = var_decl;
            } else {
                self.link_siblings(&mut first_child, &mut last_child, declarator);
            }
        }

        while self.current_token().kind == TokenKind::Punctuator && self.current_token().text == ","
        {
            self.advance();
            let declarator = self.parse_declarator()?;
            self.link_siblings(&mut first_child, &mut last_child, declarator);

            self.skip_asm_label();

            if let Some(attr_result) = self.parse_attribute_after_declarator() {
                let attr = attr_result?;
                self.link_siblings(&mut first_child, &mut last_child, attr);
            }

            self.skip_asm_label();
        }

        if self.skip_punctuator(";") {
            return Ok(self.alloc_node(20, 0, NodeOffset::NULL, first_child, NodeOffset::NULL));
        }

        let is_lbrace =
            self.current_token().kind == TokenKind::Punctuator && self.current_token().text == "{";
        if is_lbrace {
            self.advance();
            let compound = self.parse_compound_statement(None)?;
            self.link_siblings(&mut first_child, &mut last_child, compound);
            return Ok(self.alloc_node(23, 0, NodeOffset::NULL, first_child, NodeOffset::NULL));
        }

        Ok(self.alloc_node(20, 0, NodeOffset::NULL, first_child, NodeOffset::NULL))
    }

    pub fn parse_declaration_specifiers(&mut self) -> Result<NodeOffset, ParseError> {
        let mut first_spec = NodeOffset::NULL;
        let mut last_spec = NodeOffset::NULL;
        let mut has_type_specifier = false;

        loop {
            // Skip __attribute__ appearing among declaration specifiers
            if (self.current_token().kind == TokenKind::Keyword
                || self.current_token().kind == TokenKind::Identifier)
                && self.current_token().text == "__attribute__"
            {
                self.advance();
                let attr = self.parse_attribute_list()?;
                self.link_siblings(&mut first_spec, &mut last_spec, attr);
                continue;
            }
            // Skip __extension__ appearing among specifiers
            if self.current_token().text == "__extension__" {
                self.advance();
                continue;
            }
            if !(self.current_token().kind == TokenKind::Keyword
                || self.current_token().kind == TokenKind::Identifier
                    && (self.is_type_specifier()
                        || self.is_type_qualifier()
                        || self.is_storage_class_specifier()
                        || self.is_function_specifier()))
            {
                // Also check if identifier is a GNU-ish qualifier/specifier
                if self.current_token().kind == TokenKind::Identifier {
                    let txt = self.current_token().text.as_str();
                    if matches!(
                        txt,
                        "__restrict"
                            | "__restrict__"
                            | "__volatile"
                            | "__volatile__"
                            | "__const"
                            | "__const__"
                            | "__inline"
                            | "__inline__"
                            | "__signed__"
                            | "__signed"
                            | "_Atomic"
                    ) {
                        // Fall through to the qualifier/specifier handling below
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }

            if self.is_storage_class_specifier() {
                let spec = self.parse_storage_class_specifier()?;
                self.link_siblings(&mut first_spec, &mut last_spec, spec);
            } else if self.is_type_specifier() || self.is_gnu_signed_keyword() {
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
        (token.kind == TokenKind::Keyword || token.kind == TokenKind::Identifier)
            && matches!(
                token.text.as_str(),
                "const"
                    | "restrict"
                    | "volatile"
                    | "__const"
                    | "__const__"
                    | "__restrict"
                    | "__restrict__"
                    | "__volatile"
                    | "__volatile__"
                    | "_Atomic"
            )
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

        Ok(self.alloc_node(
            kind,
            0,
            NodeOffset::NULL,
            NodeOffset::NULL,
            NodeOffset::NULL,
        ))
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
            "signed" | "__signed__" | "__signed" => 12,
            "unsigned" => 13,
            "struct" => return self.parse_struct_specifier(),
            "union" => return self.parse_union_specifier(),
            "enum" => return self.parse_enum_specifier(),
            "_Bool" => 14,
            "_Complex" => 15,
            "typeof" | "__typeof__" => {
                return self.parse_typeof_expr();
            }
            _ => 2,
        };

        Ok(self.alloc_node(
            kind,
            0,
            NodeOffset::NULL,
            NodeOffset::NULL,
            NodeOffset::NULL,
        ))
    }

    fn parse_typeof_expr(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("(")?;
        let inner = if self.is_type_specifier() {
            let spec = self.parse_declaration_specifiers()?;
            self.expect(")")?;
            spec
        } else {
            let expr = self.parse_expression()?;
            self.expect(")")?;
            expr
        };
        Ok(self.alloc_node(201, 0, NodeOffset::NULL, inner, NodeOffset::NULL))
    }

    fn parse_struct_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        let mut first_member = NodeOffset::NULL;
        let mut last_member = NodeOffset::NULL;

        while (self.current_token().kind == TokenKind::Keyword
            || self.current_token().kind == TokenKind::Identifier)
            && self.current_token().text == "__attribute__"
        {
            self.advance();
            let attr = self.parse_attribute_list()?;
            self.link_siblings(&mut first_member, &mut last_member, attr);
        }

        let tag_data = if self.current_token().kind == TokenKind::Identifier {
            let name = self.current_token().text.clone();
            self.advance();
            self.arena.store_string(&name).unwrap_or(NodeOffset::NULL).0
        } else {
            0
        };

        while (self.current_token().kind == TokenKind::Keyword
            || self.current_token().kind == TokenKind::Identifier)
            && self.current_token().text == "__attribute__"
        {
            self.advance();
            let attr = self.parse_attribute_list()?;
            self.link_siblings(&mut first_member, &mut last_member, attr);
        }

        if !self.skip_punctuator("{") {
            return Ok(self.alloc_node(
                4,
                tag_data,
                NodeOffset::NULL,
                first_member,
                NodeOffset::NULL,
            ));
        }

        while !self.skip_punctuator("}") {
            if self.is_at_end() {
                break;
            }
            let member_decl = self.parse_struct_declaration()?;
            self.link_siblings(&mut first_member, &mut last_member, member_decl);
        }

        Ok(self.alloc_node(
            4,
            tag_data,
            NodeOffset::NULL,
            first_member,
            NodeOffset::NULL,
        ))
    }

    fn parse_union_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        let mut first_member = NodeOffset::NULL;
        let mut last_member = NodeOffset::NULL;

        while (self.current_token().kind == TokenKind::Keyword
            || self.current_token().kind == TokenKind::Identifier)
            && self.current_token().text == "__attribute__"
        {
            self.advance();
            let attr = self.parse_attribute_list()?;
            self.link_siblings(&mut first_member, &mut last_member, attr);
        }

        let tag_data = if self.current_token().kind == TokenKind::Identifier {
            let name = self.current_token().text.clone();
            self.advance();
            self.arena.store_string(&name).unwrap_or(NodeOffset::NULL).0
        } else {
            0
        };

        while (self.current_token().kind == TokenKind::Keyword
            || self.current_token().kind == TokenKind::Identifier)
            && self.current_token().text == "__attribute__"
        {
            self.advance();
            let attr = self.parse_attribute_list()?;
            self.link_siblings(&mut first_member, &mut last_member, attr);
        }

        if !self.skip_punctuator("{") {
            return Ok(self.alloc_node(
                5,
                tag_data,
                NodeOffset::NULL,
                first_member,
                NodeOffset::NULL,
            ));
        }

        while !self.skip_punctuator("}") {
            if self.is_at_end() {
                break;
            }
            let member_decl = self.parse_struct_declaration()?;
            self.link_siblings(&mut first_member, &mut last_member, member_decl);
        }

        Ok(self.alloc_node(
            5,
            tag_data,
            NodeOffset::NULL,
            first_member,
            NodeOffset::NULL,
        ))
    }

    fn parse_struct_declaration(&mut self) -> Result<NodeOffset, ParseError> {
        // Skip __extension__ before struct members
        if self.current_token().text == "__extension__" {
            self.advance();
        }

        let specifiers = self.parse_declaration_specifiers()?;
        let mut first_declarator = NodeOffset::NULL;
        let mut last_declarator = NodeOffset::NULL;
        let mut safety = 0;

        while !self.skip_punctuator(";") {
            safety += 1;
            if self.is_at_end() || safety > 500 {
                break;
            }
            if self.current_token().kind == TokenKind::Punctuator
                && self.current_token().text == ","
            {
                self.advance();
                continue;
            }
            // Handle anonymous bitfields: `:width` without a declarator name
            if self.current_token().kind == TokenKind::Punctuator
                && self.current_token().text == ":"
            {
                self.advance(); // skip ':'
                let width = self.parse_constant_expression()?;
                let width_val = self.arena.get(width).map(|n| if n.kind == 61 { n.data } else { 0 }).unwrap_or(0);
                let bitfield_node =
                    self.alloc_node(27, width_val, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL);
                self.link_siblings(&mut first_declarator, &mut last_declarator, bitfield_node);
                continue;
            }
            // Skip __attribute__ appearing before declarators in structs
            if self.current_token().text == "__attribute__" {
                self.advance();
                let _ = self.parse_attribute_list();
                continue;
            }
            let before = self.current;
            let declarator = self.parse_declarator()?;
            // Handle bitfield width after declarator: `name : width`
            if self.current_token().kind == TokenKind::Punctuator
                && self.current_token().text == ":"
            {
                self.advance(); // skip ':'
                let width = self.parse_constant_expression()?;
                let width_val = self.arena.get(width).map(|n| if n.kind == 61 { n.data } else { 0 }).unwrap_or(0);
                // Wrap as bitfield node (kind=27) with declarator as child
                let bitfield_node =
                    self.alloc_node(27, width_val, NodeOffset::NULL, declarator, NodeOffset::NULL);
                self.link_siblings(&mut first_declarator, &mut last_declarator, bitfield_node);
            } else {
                self.link_siblings(&mut first_declarator, &mut last_declarator, declarator);
            }
            // Skip __attribute__ after declarators in struct members
            if self.current_token().text == "__attribute__" {
                self.advance();
                let _ = self.parse_attribute_list();
            }
            if self.current == before {
                self.advance();
            }
        }

        if first_declarator != NodeOffset::NULL {
            let mut last_spec = specifiers;
            loop {
                if let Some(n) = self.arena.get(last_spec) {
                    if n.next_sibling == NodeOffset::NULL {
                        break;
                    }
                    last_spec = n.next_sibling;
                } else {
                    break;
                }
            }
            if let Some(n) = self.arena.get_mut(last_spec) {
                n.next_sibling = first_declarator;
            }
        }

        Ok(self.alloc_node(25, 0, NodeOffset::NULL, specifiers, NodeOffset::NULL))
    }

    fn parse_enum_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        let mut first_const = NodeOffset::NULL;
        let mut last_const = NodeOffset::NULL;

        // Optionally consume an enum tag name
        if self.current_token().kind == TokenKind::Identifier {
            self.advance();
        }

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

                let const_node = self.alloc_node(
                    26,
                    value,
                    NodeOffset::NULL,
                    NodeOffset::NULL,
                    NodeOffset::NULL,
                );
                let ident_node =
                    self.alloc_node(60, 0, const_node, NodeOffset::NULL, NodeOffset::NULL);
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

        Ok(self.alloc_node(
            kind,
            0,
            NodeOffset::NULL,
            NodeOffset::NULL,
            NodeOffset::NULL,
        ))
    }

    fn parse_function_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        let token = self.current_token();
        let text = token.text.clone();
        self.advance();

        let kind = match text.as_str() {
            "inline" | "__inline" | "__inline__" => 93,
            "_Noreturn" | "noreturn" | "__noreturn__" => 94,
            _ => 93,
        };

        Ok(self.alloc_node(
            kind,
            0,
            NodeOffset::NULL,
            NodeOffset::NULL,
            NodeOffset::NULL,
        ))
    }

    pub fn parse_declarator(&mut self) -> Result<NodeOffset, ParseError> {
        let mut pointer_depth: u32 = 0;

        while self.skip_punctuator("*") {
            pointer_depth += 1;
        }

        let direct_decl = self.parse_direct_declarator()?;

        if pointer_depth > 0 {
            // Store the full pointer depth in the `data` field of a single kind=7
            // node.  This avoids chaining pointer nodes via `next_sibling`, which
            // is later reused by `link_siblings` / initializer attachment and would
            // corrupt the pointer-depth information.
            let pointer_node = self.alloc_node(7, pointer_depth, NodeOffset::NULL, direct_decl, NodeOffset::NULL);
            return Ok(pointer_node);
        }

        Ok(direct_decl)
    }

    fn parse_direct_declarator(&mut self) -> Result<NodeOffset, ParseError> {
        let mut declarator = NodeOffset::NULL;

        if self.current_token().kind == TokenKind::Identifier {
            let name = self.current_token().text.clone();
            self.advance();
            let string_offset = self.arena.store_string(&name).unwrap_or(NodeOffset::NULL);
            declarator = self.alloc_node(
                60,
                string_offset.0,
                NodeOffset::NULL,
                NodeOffset::NULL,
                NodeOffset::NULL,
            );
        } else if self.skip_punctuator("(") {
            let inner = self.parse_declarator()?;
            self.expect(")")?;
            declarator = inner;
        }

        loop {
            if self.skip_punctuator("[") {
                let size = if self.current_token().text != "]" {
                    let size_expr = self.parse_constant_expression()?;
                    self.arena
                        .get(size_expr)
                        .filter(|node| node.kind == 61)
                        .map(|node| node.data)
                        .unwrap_or(0)
                } else {
                    0
                };
                self.expect("]")?;
                let arr = self.alloc_node(8, size, NodeOffset::NULL, declarator, NodeOffset::NULL);
                declarator = arr;
            } else if self.skip_punctuator("(") {
                if self.current_token().text == ")" {
                    self.advance();
                    declarator =
                        self.alloc_node(9, 0, NodeOffset::NULL, declarator, NodeOffset::NULL);
                } else {
                    let (params, is_variadic) = self.parse_parameter_list()?;
                    self.expect(")")?;
                    // Chain params as declarator(ident).next_sibling so that
                    // link_siblings in parse_external_declaration cannot overwrite them
                    // via kind=9.next_sibling.
                    if params != NodeOffset::NULL && declarator != NodeOffset::NULL {
                        if let Some(d) = self.arena.get_mut(declarator) {
                            d.next_sibling = params;
                        }
                    }
                    // data=1 if variadic, 0 otherwise
                    let va_flag = if is_variadic { 1 } else { 0 };
                    declarator =
                        self.alloc_node(9, va_flag, NodeOffset::NULL, declarator, NodeOffset::NULL);
                }
            } else {
                break;
            }
        }

        Ok(declarator)
    }

    fn parse_parameter_list(&mut self) -> Result<(NodeOffset, bool), ParseError> {
        let mut first_param = NodeOffset::NULL;
        let mut last_param = NodeOffset::NULL;
        let mut is_variadic = false;

        if self.current_token().text == ")" {
            return Ok((first_param, is_variadic));
        }

        // Handle (void) — means no parameters
        if self.current_token().text == "void" {
            let next_pos = self.current + 1;
            if next_pos < self.tokens.len() && self.tokens[next_pos].text == ")" {
                self.advance(); // skip "void"
                return Ok((first_param, is_variadic));
            }
        }

        loop {
            if self.current_token().text == ")" {
                break;
            }
            if self.current_token().text == "{" {
                break;
            }
            let param = self.parse_parameter_declaration()?;
            self.link_siblings(&mut first_param, &mut last_param, param);

            if self.current_token().text == ")" {
                break;
            }
            if !self.skip_punctuator(",") {
                break;
            }
            if self.current_token().text == "..." {
                self.advance();
                is_variadic = true;
                break;
            }
        }

        Ok((first_param, is_variadic))
    }

    fn parse_parameter_declaration(&mut self) -> Result<NodeOffset, ParseError> {
        if self.is_type_specifier() || self.is_type_qualifier() || self.is_storage_class_specifier() {
            let specifiers = self.parse_declaration_specifiers()?;
            let declarator = if self.is_declarator_start() {
                self.parse_declarator()?
            } else {
                NodeOffset::NULL
            };

            // Chain declarator as last_spec.next_sibling so that link_siblings in
            // parse_parameter_list cannot overwrite it via kind=24.next_sibling.
            if declarator != NodeOffset::NULL && specifiers != NodeOffset::NULL {
                let mut last_spec = specifiers;
                loop {
                    if let Some(n) = self.arena.get(last_spec) {
                        if n.next_sibling == NodeOffset::NULL {
                            break;
                        }
                        last_spec = n.next_sibling;
                    } else {
                        break;
                    }
                }
                if let Some(n) = self.arena.get_mut(last_spec) {
                    n.next_sibling = declarator;
                }
            }

            Ok(self.alloc_node(24, 0, NodeOffset::NULL, specifiers, NodeOffset::NULL))
        } else {
            Ok(self.alloc_node(24, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
        }
    }

    /// Walk a declarator AST node to find the identifier name (kind=60).
    fn find_declarator_name(&self, offset: NodeOffset) -> Option<String> {
        let node = self.arena.get(offset)?;
        // If this is an identifier node
        if node.kind == 60 {
            return self.arena.get_string(NodeOffset(node.data)).map(|s| s.to_string());
        }
        // Recurse into first_child (for pointer/array/function declarators)
        if node.first_child != NodeOffset::NULL {
            if let Some(name) = self.find_declarator_name(node.first_child) {
                return Some(name);
            }
        }
        // Try next_sibling
        if node.next_sibling != NodeOffset::NULL {
            if let Some(name) = self.find_declarator_name(node.next_sibling) {
                return Some(name);
            }
        }
        None
    }

    pub fn is_declarator_start(&self) -> bool {
        let token = self.current_token();
        let result = token.kind == TokenKind::Identifier
            || (token.kind == TokenKind::Punctuator && token.text == "(")
            || (token.kind == TokenKind::Punctuator && token.text == "*");
        result
    }

    fn parse_compound_statement(
        &mut self,
        parent: Option<NodeOffset>,
    ) -> Result<NodeOffset, ParseError> {
        let mut first_item = NodeOffset::NULL;
        let mut last_item = NodeOffset::NULL;
        let mut safety = 0;

        while !self.skip_punctuator("}") {
            safety += 1;
            if self.is_at_end() || safety > 10000 {
                break;
            }
            if self.skip_punctuator(";") {
                let empty =
                    self.alloc_node(48, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL);
                self.link_siblings(&mut first_item, &mut last_item, empty);
                continue;
            }

            let before = self.current;
            if self.is_type_specifier() || self.is_storage_class_specifier() {
                match self.parse_declaration() {
                    Ok(decl) => self.link_siblings(&mut first_item, &mut last_item, decl),
                    Err(_) => self.advance(),
                }
            } else {
                match self.parse_statement() {
                    Ok(stmt) => self.link_siblings(&mut first_item, &mut last_item, stmt),
                    Err(_) => self.advance(),
                }
            }
            if self.current == before {
                self.advance();
            }
        }
        Ok(self.alloc_node(40, 0, NodeOffset::NULL, first_item, NodeOffset::NULL))
    }

    pub fn parse_declaration(&mut self) -> Result<NodeOffset, ParseError> {
        // Check if this is a typedef declaration
        let is_typedef = self.current_token().text == "typedef";

        let specifiers = self.parse_declaration_specifiers()?;
        let mut first_init = NodeOffset::NULL;
        let mut last_init = NodeOffset::NULL;

        if !self.skip_punctuator(";") {
            loop {
                let declarator = self.parse_declarator()?;
                let mut init = declarator;

                // If this is a typedef declaration, extract and register the name
                if is_typedef && declarator != NodeOffset::NULL {
                    if let Some(name) = self.find_declarator_name(declarator) {
                        self.typedef_names.insert(name);
                    }
                }

                // Handle __asm__("...") after declarator
                self.skip_asm_label();

                if let Some(attr_result) = self.parse_attribute_after_declarator() {
                    let attr = attr_result?;
                    self.link_siblings(&mut first_init, &mut last_init, attr);
                }

                self.skip_asm_label();

                if self.skip_punctuator("=") {
                    if self.current_token().kind == TokenKind::Punctuator
                        && (self.current_token().text == "." || self.current_token().text == "[")
                    {
                        let init_expr = self.parse_designated_init()?;
                        // Store init_expr as declarator.next_sibling so link_siblings
                        // cannot overwrite it (link_siblings sets kind=73.next_sibling).
                        if declarator != NodeOffset::NULL {
                            if let Some(d) = self.arena.get_mut(declarator) {
                                d.next_sibling = init_expr;
                            }
                        }
                        init =
                            self.alloc_node(73, 19, NodeOffset::NULL, declarator, NodeOffset::NULL);
                    } else {
                        let init_expr = self.parse_initializer()?;
                        // Store init_expr as declarator.next_sibling so link_siblings
                        // cannot overwrite it (link_siblings sets kind=73.next_sibling).
                        if declarator != NodeOffset::NULL {
                            if let Some(d) = self.arena.get_mut(declarator) {
                                d.next_sibling = init_expr;
                            }
                        }
                        init =
                            self.alloc_node(73, 19, NodeOffset::NULL, declarator, NodeOffset::NULL);
                    }
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

        if first_init != NodeOffset::NULL {
            let mut last_spec = specifiers;
            loop {
                if let Some(n) = self.arena.get(last_spec) {
                    if n.next_sibling == NodeOffset::NULL {
                        break;
                    }
                    last_spec = n.next_sibling;
                } else {
                    break;
                }
            }
            if let Some(n) = self.arena.get_mut(last_spec) {
                n.next_sibling = first_init;
            }
        }

        Ok(self.alloc_node(21, 0, NodeOffset::NULL, specifiers, NodeOffset::NULL))
    }

    pub fn parse_initializer(&mut self) -> Result<NodeOffset, ParseError> {
        if self.skip_punctuator("{") {
            let mut first_elem = NodeOffset::NULL;
            let mut last_elem = NodeOffset::NULL;

            loop {
                let elem = if self.current_token().kind == TokenKind::Punctuator
                    && (self.current_token().text == "." || self.current_token().text == "[")
                {
                    self.parse_designated_init()?
                } else {
                    self.parse_initializer()?
                };
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

        // Handle __attribute__ before statements (e.g. __attribute__((fallthrough));)
        if (token.kind == TokenKind::Keyword || token.kind == TokenKind::Identifier)
            && token.text == "__attribute__"
        {
            self.advance();
            let _ = self.parse_attribute_list();
            self.skip_punctuator(";");
            return Ok(self.alloc_node(
                48,
                0,
                NodeOffset::NULL,
                NodeOffset::NULL,
                NodeOffset::NULL,
            ));
        }

        if token.kind == TokenKind::Keyword || token.kind == TokenKind::Identifier {
            match token.text.as_str() {
                "if" => return self.parse_if_statement(),
                "while" => return self.parse_while_statement(),
                "for" => return self.parse_for_statement(),
                "return" => return self.parse_return_statement(),
                "break" => {
                    self.advance();
                    self.skip_punctuator(";");
                    return Ok(self.alloc_node(
                        46,
                        0,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                    ));
                }
                "continue" => {
                    self.advance();
                    self.skip_punctuator(";");
                    return Ok(self.alloc_node(
                        47,
                        0,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                    ));
                }
                "switch" => return self.parse_switch_statement(),
                "goto" => return self.parse_goto_statement(),
                "do" => return self.parse_do_statement(),
                "case" => return self.parse_case_label(),
                "default" => return self.parse_default_label(),
                "__extension__" => {
                    self.advance();
                    return self.parse_statement();
                }
                "__label__" => {
                    // GCC local label declaration: __label__ name1, name2, ...;
                    self.advance();
                    while !self.skip_punctuator(";") && !self.is_at_end() {
                        self.advance();
                    }
                    return Ok(self.alloc_node(
                        48,
                        0,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                    ));
                }
                "asm" | "__asm__" | "__asm" => {
                    return self.parse_asm_stmt();
                }
                _ => {}
            }
        }

        // Check for labeled statements: `identifier :`
        if token.kind == TokenKind::Identifier {
            if let Some(next) = self.peek_token(1) {
                if next.kind == TokenKind::Punctuator && next.text == ":" {
                    let label_name = token.text.clone();
                    self.advance(); // skip label name
                    self.advance(); // skip ':'
                    // Store label name in arena so backend can resolve it
                    let label_str_offset = self
                        .arena
                        .store_string(&label_name)
                        .unwrap_or(NodeOffset::NULL);
                    // A label must be followed by a statement
                    let stmt = self.parse_statement()?;
                    // kind=51 for labeled statement, data=label name string offset
                    return Ok(self.alloc_node(
                        51,
                        label_str_offset.0,
                        NodeOffset::NULL,
                        stmt,
                        NodeOffset::NULL,
                    ));
                }
            }
        }

        self.parse_expression_statement()
    }

    fn parse_case_label(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("case")?;
        let expr = self.parse_constant_expression()?;

        // Check for GNU case range extension: case LOW ... HIGH:
        if self.current_token().kind == TokenKind::Punctuator && self.current_token().text == "..." {
            self.advance(); // skip '...'
            let high_expr = self.parse_constant_expression()?;
            self.expect(":")?;
            let stmt = self.parse_statement()?;
            // kind=54 (case_range): first_child=low_expr, data=high_expr.0, next_sibling=stmt
            return Ok(self.alloc_node(54, high_expr.0, NodeOffset::NULL, expr, stmt));
        }

        self.expect(":")?;
        let stmt = self.parse_statement()?;
        Ok(self.alloc_node(52, 0, NodeOffset::NULL, expr, stmt))
    }

    fn parse_default_label(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("default")?;
        self.expect(":")?;
        let stmt = self.parse_statement()?;
        Ok(self.alloc_node(53, 0, NodeOffset::NULL, stmt, NodeOffset::NULL))
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

        // Layout (immune to link_siblings overwriting kind=41.next_sibling):
        //   kind=41.first_child = cond_wrap(kind=0)
        //     cond_wrap.first_child = condition_expr   (expr.next_sibling not touched)
        //     cond_wrap.next_sibling = body_wrap(kind=0)
        //       body_wrap.first_child = then_stmt
        //       body_wrap.next_sibling = else_stmt
        let body_wrap = self.alloc_node(
            0,
            0,
            NodeOffset::NULL,
            then_stmt,
            else_stmt.unwrap_or(NodeOffset::NULL),
        );
        let cond_wrap = self.alloc_node(0, 0, NodeOffset::NULL, condition, body_wrap);
        Ok(self.alloc_node(41, 0, NodeOffset::NULL, cond_wrap, NodeOffset::NULL))
    }

    fn parse_while_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("while")?;
        self.expect("(")?;
        let condition = self.parse_expression()?;
        self.expect(")")?;
        let body = self.parse_statement()?;

        // Layout (immune to link_siblings overwriting kind=42.next_sibling):
        //   kind=42.first_child = cond_wrap(kind=0)
        //     cond_wrap.first_child = condition_expr   (expr.next_sibling not touched)
        //     cond_wrap.next_sibling = body
        let cond_wrap = self.alloc_node(0, 0, NodeOffset::NULL, condition, body);
        Ok(self.alloc_node(42, 0, NodeOffset::NULL, cond_wrap, NodeOffset::NULL))
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

        // Layout (immune to link_siblings overwriting kind=43.next_sibling):
        //   kind=43.first_child = init_wrap(kind=0)
        //     init_wrap.first_child = init
        //     init_wrap.next_sibling = cond_wrap(kind=0)
        //       cond_wrap.first_child = condition_expr
        //       cond_wrap.next_sibling = incr_wrap(kind=0)
        //         incr_wrap.first_child = increment
        //         incr_wrap.next_sibling = body
        let incr_wrap = self.alloc_node(0, 0, NodeOffset::NULL, increment, body);
        let cond_wrap = self.alloc_node(0, 0, NodeOffset::NULL, condition, incr_wrap);
        let init_wrap = self.alloc_node(0, 0, NodeOffset::NULL, init, cond_wrap);
        Ok(self.alloc_node(43, 0, NodeOffset::NULL, init_wrap, NodeOffset::NULL))
    }

    fn parse_do_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("do")?;
        let body = self.parse_statement()?;
        self.expect("while")?;
        self.expect("(")?;
        let condition = self.parse_expression()?;
        self.expect(")")?;
        self.skip_punctuator(";");

        // Link condition as sibling of body so it survives link_siblings in compound
        // AST layout: kind=42 data=1, first_child=body→condition
        if let Some(b) = self.arena.get_mut(body) {
            b.next_sibling = condition;
        }
        Ok(self.alloc_node(42, 1, NodeOffset::NULL, body, NodeOffset::NULL))
    }

    fn parse_switch_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("switch")?;
        self.expect("(")?;
        let condition = self.parse_expression()?;
        self.expect(")")?;
        let body = self.parse_statement()?;

        Ok(self.alloc_node(50, 0, NodeOffset::NULL, condition, body))
    }

    fn parse_goto_statement(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("goto")?;
        if self.current_token().text == "*" {
            // Computed goto: goto *expr;
            self.advance(); // skip '*'
            let expr = self.parse_expression()?;
            self.skip_punctuator(";");
            // kind=49 with data=0 (no label name), first_child=expr for computed goto
            Ok(self.alloc_node(49, 0, NodeOffset::NULL, expr, NodeOffset::NULL))
        } else if self.current_token().kind == TokenKind::Identifier {
            let label = self.current_token().text.clone();
            self.advance();
            self.skip_punctuator(";");
            // Store label name in arena
            let label_str_offset = self
                .arena
                .store_string(&label)
                .unwrap_or(NodeOffset::NULL);
            Ok(self.alloc_node(
                49,
                label_str_offset.0,
                NodeOffset::NULL,
                NodeOffset::NULL,
                NodeOffset::NULL,
            ))
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

    pub fn parse_expression(&mut self) -> Result<NodeOffset, ParseError> {
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

    pub fn parse_assignment_expression(&mut self) -> Result<NodeOffset, ParseError> {
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
        let mut left = self.parse_cast_expression()?;

        loop {
            let (op_prec, op_code) = self.get_binary_operator();
            if op_prec == 0 || op_prec <= precedence {
                break;
            }

            self.advance();
            let right = self.parse_binary_op(op_prec - 1)?;
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
            "==" => (6, 6),
            "!=" => (6, 7),
            "<" => (7, 8),
            ">" => (7, 9),
            "<=" => (7, 10),
            ">=" => (7, 11),
            "<<" => (8, 17),
            ">>" => (8, 18),
            "+" => (9, 1),
            "-" => (9, 2),
            "*" => (10, 3),
            "/" => (10, 4),
            "%" => (10, 5),
            _ => (0, 0),
        }
    }

    fn parse_unary_expression(&mut self) -> Result<NodeOffset, ParseError> {
        let token = self.current_token();

        // sizeof is a keyword, not a punctuator — check it first
        if token.kind == TokenKind::Keyword && token.text == "sizeof" {
            return self.parse_sizeof_expression();
        }

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
                    let operand = self.parse_unary_expression()?;
                    return Ok(self.alloc_node(65, 0, NodeOffset::NULL, operand, NodeOffset::NULL));
                }
                "-" => {
                    self.advance();
                    let operand = self.parse_unary_expression()?;
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
                _ => {}
            }
        }

        self.parse_postfix_expression()
    }

    fn parse_sizeof_expression(&mut self) -> Result<NodeOffset, ParseError> {
        self.advance();

        if self.skip_punctuator("(") {
            if self.is_type_specifier() {
                let specifiers = self.parse_declaration_specifiers()?;
                self.expect(")")?;
                Ok(self.alloc_node(71, 0, NodeOffset::NULL, specifiers, NodeOffset::NULL))
            } else {
                let expr = self.parse_expression()?;
                self.expect(")")?;
                Ok(self.alloc_node(71, 1, NodeOffset::NULL, expr, NodeOffset::NULL))
            }
        } else {
            let operand = self.parse_unary_expression()?;
            Ok(self.alloc_node(71, 1, NodeOffset::NULL, operand, NodeOffset::NULL))
        }
    }

    fn parse_cast_expression(&mut self) -> Result<NodeOffset, ParseError> {
        if self.skip_punctuator("(") {
            if self.is_type_specifier() || self.is_type_qualifier() {
                let type_spec = self.parse_declaration_specifiers()?;
                // Handle abstract declarator (pointer/array part of cast type)
                let mut cast_type = type_spec;
                if self.current_token().text == "*" {
                    let ptr_decl = self.parse_declarator()?;
                    if ptr_decl != NodeOffset::NULL {
                        // Chain declarator onto the specifier chain
                        let mut last = cast_type;
                        loop {
                            let ns = self.arena.get(last).map(|n| n.next_sibling).unwrap_or(NodeOffset::NULL);
                            if ns == NodeOffset::NULL { break; }
                            last = ns;
                        }
                        if let Some(n) = self.arena.get_mut(last) {
                            n.next_sibling = ptr_decl;
                        }
                    }
                }
                if self.skip_punctuator(")") {
                    // Compound literal: (type_name){initializer_list}
                    if self.current_token().kind == TokenKind::Punctuator
                        && self.current_token().text == "{"
                    {
                        let init_list = self.parse_initializer()?;
                        return Ok(self.alloc_node(
                            212,
                            init_list.0,
                            NodeOffset::NULL,
                            cast_type,
                            NodeOffset::NULL,
                        ));
                    }
                    let expr = self.parse_cast_expression()?;
                    return Ok(self.alloc_node(70, 0, NodeOffset::NULL, cast_type, expr));
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
                if args != NodeOffset::NULL {
                    if let Some(callee) = self.arena.get_mut(expr) {
                        callee.next_sibling = args;
                    }
                }
                expr = self.alloc_node(67, 0, NodeOffset::NULL, expr, NodeOffset::NULL);
            } else if self.skip_punctuator(".") {
                if self.current_token().kind == TokenKind::Identifier {
                    let member = self.current_token().text.clone();
                    self.advance();
                    let member_offset =
                        self.arena.store_string(&member).unwrap_or(NodeOffset::NULL);
                    let member_node = self.alloc_node(
                        60,
                        member_offset.0,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                    );
                    expr = self.alloc_node(69, 0, NodeOffset::NULL, expr, member_node);
                }
            } else if self.skip_punctuator("->") {
                if self.current_token().kind == TokenKind::Identifier {
                    let member = self.current_token().text.clone();
                    self.advance();
                    let member_offset =
                        self.arena.store_string(&member).unwrap_or(NodeOffset::NULL);
                    let member_node = self.alloc_node(
                        60,
                        member_offset.0,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                    );
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
                // Wrap each argument in an arg-wrapper node (kind=74) to isolate
                // expression-internal next_sibling chains from the argument list chain.
                let wrapper = self.alloc_node(74, 0, NodeOffset::NULL, arg, NodeOffset::NULL);
                self.link_siblings(&mut first_arg, &mut last_arg, wrapper);

                if self.current_token().text == ")" {
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

        if token.kind == TokenKind::Punctuator && token.text == "(" {
            if self
                .peek_token(1)
                .map(|t| t.kind == TokenKind::Punctuator && t.text == "{")
                .unwrap_or(false)
            {
                return self.parse_statement_expr();
            }
        }

        if token.kind == TokenKind::Keyword || token.kind == TokenKind::Identifier {
            if token.text == "typeof" || token.text == "__typeof__" {
                return self.parse_typeof_expr();
            }
            if token.text == "__extension__" {
                return self.parse_extension_wrapper();
            }
            if token.text.starts_with("__builtin_") {
                let name = token.text.clone();
                self.advance();
                return self.parse_builtin_call(&name);
            }
        }

        if token.kind == TokenKind::Punctuator && token.text == "&" {
            if self
                .peek_token(1)
                .map(|t| t.kind == TokenKind::Punctuator && t.text == "&")
                .unwrap_or(false)
            {
                return self.parse_label_addr();
            }
        }

        match token.kind {
            TokenKind::Identifier => {
                let name = token.text.clone();
                self.advance();
                if name.starts_with("__builtin_") {
                    return self.parse_builtin_call(&name);
                }
                let string_offset = self.arena.store_string(&name).unwrap_or(NodeOffset::NULL);
                Ok(self.alloc_node(
                    60,
                    string_offset.0,
                    NodeOffset::NULL,
                    NodeOffset::NULL,
                    NodeOffset::NULL,
                ))
            }
            TokenKind::IntConstant => {
                let value = token.text.parse::<u32>().unwrap_or(0);
                self.advance();
                Ok(self.alloc_node(
                    61,
                    value,
                    NodeOffset::NULL,
                    NodeOffset::NULL,
                    NodeOffset::NULL,
                ))
            }
            TokenKind::CharConstant => {
                let text = token.text.clone();
                self.advance();
                // Parse char constant value: 'x' -> x as u32
                let char_val = if text.len() >= 3 && text.starts_with('\'') && text.ends_with('\'') {
                    let inner = &text[1..text.len()-1];
                    if inner.starts_with('\\') && inner.len() >= 2 {
                        match inner.as_bytes()[1] {
                            b'n' => 10u32,
                            b't' => 9,
                            b'r' => 13,
                            b'0' => 0,
                            b'\\' => 92,
                            b'\'' => 39,
                            b'"' => 34,
                            b'a' => 7,
                            b'b' => 8,
                            b'f' => 12,
                            b'v' => 11,
                            b'x' => u32::from_str_radix(&inner[2..], 16).unwrap_or(0),
                            c => c as u32,
                        }
                    } else if !inner.is_empty() {
                        inner.as_bytes()[0] as u32
                    } else {
                        0
                    }
                } else {
                    0
                };
                Ok(self.alloc_node(62, char_val, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
            }
            TokenKind::StringLiteral => {
                let text = token.text.clone();
                self.advance();
                // Strip quotes and process escape sequences
                let inner = if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
                    let raw = &text[1..text.len()-1];
                    let mut result = String::new();
                    let mut chars = raw.chars();
                    while let Some(c) = chars.next() {
                        if c == '\\' {
                            match chars.next() {
                                Some('n') => result.push('\n'),
                                Some('t') => result.push('\t'),
                                Some('r') => result.push('\r'),
                                Some('0') => result.push('\0'),
                                Some('\\') => result.push('\\'),
                                Some('\'') => result.push('\''),
                                Some('"') => result.push('"'),
                                Some('a') => result.push('\x07'),
                                Some('b') => result.push('\x08'),
                                Some('f') => result.push('\x0C'),
                                Some('v') => result.push('\x0B'),
                                Some(other) => { result.push('\\'); result.push(other); }
                                None => result.push('\\'),
                            }
                        } else {
                            result.push(c);
                        }
                    }
                    result
                } else {
                    text.clone()
                };
                // Concatenate adjacent string literals
                let mut full_string = inner;
                while self.current < self.tokens.len() && self.tokens[self.current].kind == TokenKind::StringLiteral {
                    let next_text = self.tokens[self.current].text.clone();
                    self.advance();
                    if next_text.len() >= 2 && next_text.starts_with('"') && next_text.ends_with('"') {
                        let raw = &next_text[1..next_text.len()-1];
                        let mut chars = raw.chars();
                        while let Some(c) = chars.next() {
                            if c == '\\' {
                                match chars.next() {
                                    Some('n') => full_string.push('\n'),
                                    Some('t') => full_string.push('\t'),
                                    Some('r') => full_string.push('\r'),
                                    Some('0') => full_string.push('\0'),
                                    Some('\\') => full_string.push('\\'),
                                    Some(other) => { full_string.push('\\'); full_string.push(other); }
                                    None => full_string.push('\\'),
                                }
                            } else {
                                full_string.push(c);
                            }
                        }
                    }
                }
                let string_offset = self.arena.store_string(&full_string).unwrap_or(NodeOffset::NULL);
                Ok(self.alloc_node(63, string_offset.0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL))
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

    pub fn parse_constant_expression(&mut self) -> Result<NodeOffset, ParseError> {
        self.parse_conditional_expression()
    }

    pub fn link_siblings(
        &mut self,
        first: &mut NodeOffset,
        last: &mut NodeOffset,
        node: NodeOffset,
    ) {
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

    fn parse_attribute_after_declarator(&mut self) -> Option<Result<NodeOffset, ParseError>> {
        if self.current_token().kind == TokenKind::Keyword
            && self.current_token().text == "__attribute__"
        {
            Some(self.parse_attribute_list())
        } else if self.current_token().kind == TokenKind::Identifier
            && self.current_token().text == "__attribute__"
        {
            Some(self.parse_attribute_list())
        } else {
            None
        }
    }

    /// Skip __asm__("..." "...") or asm("...") labels used by GCC for symbol redirects.
    pub fn skip_asm_label(&mut self) {
        if (self.current_token().kind == TokenKind::Keyword
            || self.current_token().kind == TokenKind::Identifier)
            && matches!(
                self.current_token().text.as_str(),
                "__asm__" | "__asm" | "asm"
            )
        {
            self.advance(); // skip __asm__
            if self.skip_punctuator("(") {
                // Skip everything inside the parens, handling nested parens
                let mut depth = 1;
                let mut asm_safety = 0;
                while depth > 0 && !self.is_at_end() {
                    asm_safety += 1;
                    if asm_safety > 200 {
                        break;
                    }
                    if self.current_token().text == "(" {
                        depth += 1;
                    } else if self.current_token().text == ")" {
                        depth -= 1;
                        if depth == 0 {
                            self.advance(); // skip final ')'
                            break;
                        }
                    }
                    self.advance();
                }
            }
        }
    }

    fn parse_designated_init(&mut self) -> Result<NodeOffset, ParseError> {
        if self.skip_punctuator(".") {
            if self.current_token().kind == TokenKind::Identifier {
                let field = self.current_token().text.clone();
                self.advance();
                let field_offset = self.arena.store_string(&field).unwrap_or(NodeOffset::NULL);

                self.expect("=")?;
                let value = self.parse_initializer()?;

                return Ok(self.alloc_node(
                    205,
                    field_offset.0,
                    NodeOffset::NULL,
                    value,
                    NodeOffset::NULL,
                ));
            }
        } else if self.skip_punctuator("[") {
            let index = self.parse_constant_expression()?;
            self.expect("]")?;
            self.expect("=")?;
            let value = self.parse_initializer()?;

            return Ok(self.alloc_node(205, 1, NodeOffset::NULL, index, value));
        }

        self.parse_initializer()
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

#[cfg(test)]
mod integration_tests {
    use super::*;
    use crate::db::OpticDb;
    use crate::frontend::preprocessor::Preprocessor;
    use std::fs;
    use tempfile::{NamedTempFile, TempDir};

    fn create_test_env() -> (TempDir, String) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.redb");
        (temp_dir, db_path.to_str().unwrap().to_string())
    }

    #[test]
    fn test_parse_simple_c_file_through_preprocessor() {
        let (temp_dir, db_path) = create_test_env();
        let c_file = temp_dir.path().join("simple.c");
        fs::write(&c_file, "int main() { return 0; }").unwrap();

        let db = OpticDb::new(&db_path).unwrap();
        let mut pp = Preprocessor::new(db);
        let tokens = pp.process(c_file.to_str().unwrap()).unwrap();

        let arena_file = temp_dir.path().join("arena.bin");
        let arena = Arena::new(arena_file.to_str().unwrap(), 1024).unwrap();
        let mut parser = Parser::new(arena);
        let result = parser.parse_tokens(tokens);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_file_with_define_macros() {
        let (temp_dir, db_path) = create_test_env();
        let c_file = temp_dir.path().join("macro.c");
        fs::write(&c_file, "#define MAX 100\nint x = MAX;").unwrap();

        let db = OpticDb::new(&db_path).unwrap();
        let mut pp = Preprocessor::new(db);
        let tokens = pp.process(c_file.to_str().unwrap()).unwrap();

        let arena_file = temp_dir.path().join("arena.bin");
        let arena = Arena::new(arena_file.to_str().unwrap(), 1024).unwrap();
        let mut parser = Parser::new(arena);
        let result = parser.parse_tokens(tokens);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_file_with_ifdef_conditionals() {
        let (temp_dir, db_path) = create_test_env();
        let c_file = temp_dir.path().join("conditional.c");
        fs::write(
            &c_file,
            "#define DEBUG\n#ifdef DEBUG\nint debug_val = 1;\n#endif",
        )
        .unwrap();

        let db = OpticDb::new(&db_path).unwrap();
        let mut pp = Preprocessor::new(db);
        let tokens = pp.process(c_file.to_str().unwrap()).unwrap();

        let arena_file = temp_dir.path().join("arena.bin");
        let arena = Arena::new(arena_file.to_str().unwrap(), 1024).unwrap();
        let mut parser = Parser::new(arena);
        let result = parser.parse_tokens(tokens);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_file_with_include() {
        let (temp_dir, db_path) = create_test_env();
        let header = temp_dir.path().join("header.h");
        fs::write(&header, "int global_var;").unwrap();

        let c_file = temp_dir.path().join("with_include.c");
        fs::write(
            &c_file,
            "#include \"header.h\"\nint main() { return global_var; }",
        )
        .unwrap();

        let db = OpticDb::new(&db_path).unwrap();
        let mut pp = Preprocessor::new(db);
        pp.add_include_path(temp_dir.path().to_str().unwrap());
        let tokens = pp.process(c_file.to_str().unwrap()).unwrap();

        let arena_file = temp_dir.path().join("arena.bin");
        let arena = Arena::new(arena_file.to_str().unwrap(), 1024).unwrap();
        let mut parser = Parser::new(arena);
        let result = parser.parse_tokens(tokens);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_file_with_nested_includes() {
        let (temp_dir, db_path) = create_test_env();
        let inner = temp_dir.path().join("inner.h");
        fs::write(&inner, "int inner_val = 42;").unwrap();

        let outer = temp_dir.path().join("outer.h");
        fs::write(&outer, "#include \"inner.h\"\nint outer_val = 1;").unwrap();

        let c_file = temp_dir.path().join("nested.c");
        fs::write(
            &c_file,
            "#include \"outer.h\"\nint main() { return inner_val + outer_val; }",
        )
        .unwrap();

        let db = OpticDb::new(&db_path).unwrap();
        let mut pp = Preprocessor::new(db);
        pp.add_include_path(temp_dir.path().to_str().unwrap());
        let tokens = pp.process(c_file.to_str().unwrap()).unwrap();

        let arena_file = temp_dir.path().join("arena.bin");
        let arena = Arena::new(arena_file.to_str().unwrap(), 1024).unwrap();
        let mut parser = Parser::new(arena);
        let result = parser.parse_tokens(tokens);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_backwards_compatible_parse_method() {
        let temp_file = NamedTempFile::new().unwrap();
        let arena = Arena::new(temp_file.path(), 1024).unwrap();
        let mut parser = Parser::new(arena);

        let source = "void foo() { int x = 1; }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }
}
