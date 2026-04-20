use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset, SourceLocation};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    EndOfFile,
    Identifier,
    Number,
    StringLiteral,
    Punctuator,
    Whitespace,
    Hash,
    HashHash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub offset: u32,
    pub length: u32,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MacroDefinition {
    ObjectLike {
        replacement: Vec<Token>,
    },
    FunctionLike {
        params: Vec<String>,
        is_variadic: bool,
        replacement: Vec<Token>,
    },
}

pub struct MacroInvocation {
    pub name: String,
    pub args: Option<Vec<Vec<Token>>>,
    pub invocation_offset: NodeOffset,
}

pub struct MacroExpander<'a> {
    arena: &'a mut Arena,
    definitions: HashMap<String, MacroDefinition>,
    string_interner: HashMap<String, u32>,
    string_data: Vec<u8>,
    active_macros: Vec<String>,
}

impl<'a> MacroExpander<'a> {
    pub fn new(arena: &'a mut Arena) -> Self {
        Self {
            arena,
            definitions: HashMap::new(),
            string_interner: HashMap::new(),
            string_data: Vec::new(),
            active_macros: Vec::new(),
        }
    }

    pub fn define_macro(&mut self, name: &str, definition: MacroDefinition) {
        self.definitions.insert(name.to_string(), definition);
    }

    pub fn is_macro_defined(&self, name: &str) -> bool {
        self.definitions.contains_key(name)
    }

    pub fn get_macro_definition(&self, name: &str) -> Option<&MacroDefinition> {
        self.definitions.get(name)
    }

    fn intern_string(&mut self, s: &str) -> u32 {
        if let Some(&offset) = self.string_interner.get(s) {
            return offset;
        }
        let offset = self.string_data.len() as u32;
        self.string_data.extend_from_slice(s.as_bytes());
        self.string_data.push(0);
        self.string_interner.insert(s.to_string(), offset);
        offset
    }

    pub fn expand_macros(&mut self, tokens: &[Token], source: &str) -> Vec<Token> {
        let mut expanded = Vec::new();
        let mut i = 0;

        while i < tokens.len() {
            let token = &tokens[i];

            match token.kind {
                TokenKind::Identifier => {
                    let identifier = self.extract_token_text(token, source);

                    if let Some(&next_kind) = tokens.get(i + 1).map(|t| &t.kind) {
                        if next_kind == TokenKind::Whitespace || next_kind == TokenKind::Punctuator
                        {
                            let next_text = tokens
                                .get(i + 1)
                                .map(|t| self.extract_token_text(t, source))
                                .unwrap_or_default();

                            if next_text == "(" && self.is_macro_defined(&identifier) {
                                if let Some(expanded_tokens) =
                                    self.expand_function_macro(&identifier, tokens, &mut i, source)
                                {
                                    expanded.extend(expanded_tokens);
                                    continue;
                                }
                            }
                        }
                    }

                    if self.is_macro_defined(&identifier) && !self.is_active_macro(&identifier) {
                        let expanded_tokens = self.expand_object_macro(&identifier, source);
                        expanded.extend(expanded_tokens);
                    } else {
                        expanded.push(*token);
                    }
                }
                TokenKind::Hash if !self.is_stringification_or_paste(tokens, i) => {
                    if let Some((stringified, new_i)) =
                        self.handle_stringification(tokens, i, source)
                    {
                        expanded.push(stringified);
                        i = new_i;
                    } else {
                        expanded.push(*token);
                        i += 1;
                    }
                }
                _ => {
                    expanded.push(*token);
                    i += 1;
                }
            }
        }

        expanded
    }

    fn is_stringification_or_paste(&self, tokens: &[Token], i: usize) -> bool {
        let remaining = &tokens[i..];
        if remaining.len() < 2 {
            return false;
        }
        let next_kind = remaining[1].kind;
        next_kind == TokenKind::Hash || next_kind == TokenKind::HashHash
    }

    fn handle_stringification(
        &self,
        tokens: &[Token],
        i: usize,
        source: &str,
    ) -> Option<(Token, usize)> {
        if i + 1 >= tokens.len() {
            return None;
        }

        let _hash_token = tokens[i];
        let next_token = &tokens[i + 1];

        if next_token.kind == TokenKind::HashHash {
            return None;
        }

        if next_token.kind == TokenKind::Identifier {
            let _arg_name = self.extract_token_text(&tokens[i + 1], source);
            let stringified = self.stringify_argument(&tokens[i + 1], source);
            Some((stringified, i + 2))
        } else {
            None
        }
    }

    fn stringify_argument(&self, token: &Token, source: &str) -> Token {
        let text = self.extract_token_text(token, source);
        let quoted = format!("\"{}\"", text.replace("\"", "\\\""));
        Token {
            kind: TokenKind::StringLiteral,
            offset: token.offset,
            length: quoted.len() as u32,
            line: token.line,
            column: token.column,
        }
    }

    fn expand_object_macro(&mut self, name: &str, source: &str) -> Vec<Token> {
        let replacement = match self.definitions.get(name).unwrap() {
            MacroDefinition::ObjectLike { replacement } => replacement.clone(),
            MacroDefinition::FunctionLike { .. } => return Vec::new(),
        };

        self.active_macros.push(name.to_string());
        let expanded = self.substitute_tokens(&replacement, &[], source);
        self.active_macros.pop();
        expanded
    }

    fn expand_function_macro(
        &mut self,
        name: &str,
        tokens: &[Token],
        i: &mut usize,
        source: &str,
    ) -> Option<Vec<Token>> {
        let definition = self.definitions.get(name)?.clone();

        let _params = match &definition {
            MacroDefinition::FunctionLike { params, .. } => params.clone(),
            MacroDefinition::ObjectLike { .. } => return None,
        };

        let mut arg_tokens = Vec::new();
        let mut paren_depth = 0;
        let mut current_arg = Vec::new();
        let mut j = *i + 2;

        while j < tokens.len() {
            let token = &tokens[j];
            let text = self.extract_token_text(token, source);

            match text.as_str() {
                "(" => {
                    paren_depth += 1;
                    current_arg.push(*token);
                }
                ")" if paren_depth == 0 => {
                    if !current_arg.is_empty() {
                        arg_tokens.push(current_arg);
                    }
                    break;
                }
                "," if paren_depth == 0 => {
                    arg_tokens.push(current_arg);
                    current_arg = Vec::new();
                }
                _ => {
                    current_arg.push(*token);
                }
            }

            if paren_depth > 0 && text == ")" {
                paren_depth -= 1;
            }

            j += 1;
        }

        if arg_tokens.is_empty() {
            return None;
        }

        *i = j;

        self.active_macros.push(name.to_string());
        let expanded = self.substitute_tokens_with_args(&definition, &arg_tokens, source);
        self.active_macros.pop();

        Some(expanded)
    }

    fn substitute_tokens(
        &mut self,
        tokens: &[Token],
        _args: &[Vec<Token>],
        source: &str,
    ) -> Vec<Token> {
        let mut result = Vec::new();
        let mut i = 0;

        while i < tokens.len() {
            let token = &tokens[i];

            if token.kind == TokenKind::HashHash {
                if result.len() > 0 && i + 1 < tokens.len() {
                    let last = result.pop().unwrap();
                    let next = &tokens[i + 1];
                    let pasted = self.paste_tokens(last, *next, source);
                    result.push(pasted);
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }

            if token.kind == TokenKind::Hash && i + 1 < tokens.len() {
                let next = &tokens[i + 1];
                let stringified = self.stringify_argument(next, source);
                result.push(stringified);
                i += 2;
                continue;
            }

            result.push(*token);
            i += 1;
        }

        result
    }

    fn substitute_tokens_with_args(
        &mut self,
        definition: &MacroDefinition,
        args: &[Vec<Token>],
        source: &str,
    ) -> Vec<Token> {
        let (params, replacement) = match definition {
            MacroDefinition::FunctionLike {
                params,
                replacement,
                ..
            } => (params, replacement),
            MacroDefinition::ObjectLike { .. } => return Vec::new(),
        };

        let mut result = Vec::new();
        let mut i = 0;

        while i < replacement.len() {
            let token = &replacement[i];

            if token.kind == TokenKind::HashHash {
                if result.len() > 0 && i + 1 < replacement.len() {
                    let last = result.pop().unwrap();
                    let next = &replacement[i + 1];
                    let pasted = self.paste_tokens(last, *next, source);
                    result.push(pasted);
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }

            if token.kind == TokenKind::Hash && i + 1 < replacement.len() {
                let next = &replacement[i + 1];
                if next.kind == TokenKind::Identifier {
                    let arg_name = self.extract_token_text(next, source);
                    if let Some(param_idx) = params.iter().position(|p| p == &arg_name) {
                        if param_idx < args.len() {
                            let stringified = self.stringify_tokens(&args[param_idx], source);
                            result.push(stringified);
                            i += 2;
                            continue;
                        }
                    }
                }
            }

            if token.kind == TokenKind::Identifier {
                let token_text = self.extract_token_text(token, source);
                if let Some(param_idx) = params.iter().position(|p| p == &token_text) {
                    if param_idx < args.len() {
                        let mut substituted = self.expand_macros(&args[param_idx], source);
                        result.append(&mut substituted);
                        i += 1;
                        continue;
                    }
                }
            }

            result.push(*token);
            i += 1;
        }

        result
    }

    fn stringify_tokens(&self, tokens: &[Token], source: &str) -> Token {
        let mut text = String::new();
        for (j, token) in tokens.iter().enumerate() {
            if j > 0 && token.kind != TokenKind::Whitespace {
                text.push(' ');
            }
            text.push_str(&self.extract_token_text(token, source));
        }
        let quoted = format!("\"{}\"", text.replace("\"", "\\\""));
        Token {
            kind: TokenKind::StringLiteral,
            offset: tokens.first().map(|t| t.offset).unwrap_or(0),
            length: quoted.len() as u32,
            line: tokens.first().map(|t| t.line).unwrap_or(0),
            column: tokens.first().map(|t| t.column).unwrap_or(0),
        }
    }

    fn paste_tokens(&self, left: Token, right: Token, source: &str) -> Token {
        let left_text = self.extract_token_text(&left, source);
        let right_text = self.extract_token_text(&right, source);
        let pasted = format!("{}{}", left_text, right_text);

        let kind = if pasted
            .chars()
            .next()
            .map(|c| c.is_alphabetic())
            .unwrap_or(false)
        {
            TokenKind::Identifier
        } else if pasted
            .chars()
            .next()
            .map(|c| c.is_numeric())
            .unwrap_or(false)
        {
            TokenKind::Number
        } else {
            TokenKind::Punctuator
        };

        Token {
            kind,
            offset: left.offset,
            length: pasted.len() as u32,
            line: left.line,
            column: left.column,
        }
    }

    fn extract_token_text(&self, token: &Token, source: &str) -> String {
        let start = token.offset as usize;
        let end = start + token.length as usize;
        if start < source.len() && end <= source.len() {
            source[start..end].to_string()
        } else {
            String::new()
        }
    }

    fn is_active_macro(&self, name: &str) -> bool {
        self.active_macros.iter().any(|s| s.as_str() == name)
    }

    pub fn build_expanded_ast(&mut self, tokens: &[Token], source: &str) -> NodeOffset {
        let root = self
            .arena
            .alloc(CAstNode {
                kind: 0,
                flags: NodeFlags::IS_VALID,
                parent: NodeOffset::NULL,
                first_child: NodeOffset::NULL,
                last_child: NodeOffset::NULL,
                next_sibling: NodeOffset::NULL,
                prev_sibling: NodeOffset::NULL,
                child_count: 0,
                data: 0,
                source: SourceLocation::unknown(),
                payload_offset: NodeOffset::NULL,
                payload_len: 0,
            })
            .unwrap_or(NodeOffset::NULL);

        if root == NodeOffset::NULL {
            return root;
        }

        let expanded_tokens = self.expand_macros(tokens, source);
        let mut last_child = NodeOffset::NULL;

        for token in &expanded_tokens {
            let string_offset = self.intern_string(&self.extract_token_text(token, source));
            let token_node = self
                .arena
                .alloc(CAstNode {
                    kind: token.kind as u16,
                    flags: NodeFlags::IS_VALID,
                    parent: root,
                    first_child: NodeOffset::NULL,
                    last_child: NodeOffset::NULL,
                    next_sibling: NodeOffset::NULL,
                    prev_sibling: NodeOffset::NULL,
                    child_count: 0,
                    data: string_offset,
                    source: SourceLocation::unknown(),
                    payload_offset: NodeOffset::NULL,
                    payload_len: 0,
                })
                .unwrap_or(NodeOffset::NULL);

            if token_node != NodeOffset::NULL {
                if last_child == NodeOffset::NULL {
                    if let Some(root_node) = self.arena.get_mut(root) {
                        root_node.first_child = token_node;
                    }
                } else {
                    if let Some(sibling) = self.arena.get_mut(last_child) {
                        sibling.next_sibling = token_node;
                    }
                }
                last_child = token_node;
            }
        }

        root
    }

    pub fn expand_to_dual_node(
        &mut self,
        invocation_offset: NodeOffset,
        name: &str,
        args: Option<&[Vec<Token>]>,
        source: &str,
    ) -> NodeOffset {
        let expanded_kind = 256;

        let expanded_node = self
            .arena
            .alloc(CAstNode {
                kind: expanded_kind,
                flags: NodeFlags::IS_VALID | NodeFlags::HAS_ERROR,
                parent: NodeOffset::NULL,
                first_child: NodeOffset::NULL,
                last_child: NodeOffset::NULL,
                next_sibling: NodeOffset::NULL,
                prev_sibling: NodeOffset::NULL,
                child_count: 0,
                data: 0,
                source: SourceLocation::unknown(),
                payload_offset: NodeOffset::NULL,
                payload_len: 0,
            })
            .unwrap_or(NodeOffset::NULL);

        if expanded_node == NodeOffset::NULL {
            return NodeOffset::NULL;
        }

        if let Some(inv_node) = self.arena.get_mut(invocation_offset) {
            inv_node.data = expanded_node.0;
            inv_node.flags |= NodeFlags::IS_VALID;
        }

        let expanded_tokens = if let Some(def) = self.get_macro_definition(name).cloned() {
            match def {
                MacroDefinition::ObjectLike { replacement } => {
                    self.substitute_tokens(&replacement, &[], source)
                }
                MacroDefinition::FunctionLike { .. } => {
                    if let Some(args) = args {
                        self.substitute_tokens_with_args(&def, args, source)
                    } else {
                        Vec::new()
                    }
                }
            }
        } else {
            Vec::new()
        };

        let mut last_child = NodeOffset::NULL;
        for token in &expanded_tokens {
            let string_offset = self.intern_string(&self.extract_token_text(token, source));
            let token_node = self
                .arena
                .alloc(CAstNode {
                    kind: token.kind as u16,
                    flags: NodeFlags::IS_VALID,
                    parent: expanded_node,
                    first_child: NodeOffset::NULL,
                    last_child: NodeOffset::NULL,
                    next_sibling: NodeOffset::NULL,
                    prev_sibling: NodeOffset::NULL,
                    child_count: 0,
                    data: string_offset,
                    source: SourceLocation::unknown(),
                    payload_offset: NodeOffset::NULL,
                    payload_len: 0,
                })
                .unwrap_or(NodeOffset::NULL);

            if token_node != NodeOffset::NULL {
                if last_child == NodeOffset::NULL {
                    if let Some(exp_node) = self.arena.get_mut(expanded_node) {
                        exp_node.first_child = token_node;
                    }
                } else {
                    if let Some(sibling) = self.arena.get_mut(last_child) {
                        sibling.next_sibling = token_node;
                    }
                }
                last_child = token_node;
            }
        }

        if let Some(exp_node) = self.arena.get_mut(expanded_node) {
            exp_node.flags -= NodeFlags::HAS_ERROR;
        }

        expanded_node
    }

    pub fn get_invocation_link(&self, expanded_node: NodeOffset) -> Option<NodeOffset> {
        if expanded_node == NodeOffset::NULL {
            return None;
        }

        let node = *self.arena.get(expanded_node)?;

        if node.kind == 256 {
            Some(NodeOffset(node.data))
        } else {
            None
        }
    }
}

pub struct Lexer {
    source: String,
    offset: u32,
    line: u32,
    column: u32,
}

impl Lexer {
    pub fn new(source: String) -> Self {
        Self {
            source,
            offset: 0,
            line: 1,
            column: 1,
        }
    }

    pub fn tokenize(&mut self) -> Vec<Token> {
        let mut tokens = Vec::new();

        while self.offset < self.source.len() as u32 {
            let c = self.current_char();

            match c {
                ' ' | '\t' | '\r' => {
                    tokens.push(self.make_token(TokenKind::Whitespace));
                    self.advance();
                }
                '\n' => {
                    tokens.push(self.make_token(TokenKind::Whitespace));
                    self.newline();
                }
                '#' => {
                    if self.peek_char() == Some('#') {
                        tokens.push(Token {
                            kind: TokenKind::HashHash,
                            offset: self.offset,
                            length: 2,
                            line: self.line,
                            column: self.column,
                        });
                        self.advance();
                        self.advance();
                    } else {
                        tokens.push(self.make_token(TokenKind::Hash));
                        self.advance();
                    }
                }
                '"' => {
                    let start = self.offset;
                    self.advance();
                    while self.offset < self.source.len() as u32 && self.current_char() != '"' {
                        if self.current_char() == '\\' && self.peek_char() == Some('"') {
                            self.advance();
                        }
                        self.advance();
                    }
                    if self.current_char() == '"' {
                        self.advance();
                    }
                    tokens.push(Token {
                        kind: TokenKind::StringLiteral,
                        offset: start,
                        length: self.offset - start,
                        line: self.line,
                        column: self.column,
                    });
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    let start = self.offset;
                    while self.offset < self.source.len() as u32 {
                        let c = self.current_char();
                        if c.is_alphanumeric() || c == '_' {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token {
                        kind: TokenKind::Identifier,
                        offset: start,
                        length: self.offset - start,
                        line: self.line,
                        column: self.column,
                    });
                }
                '0'..='9' => {
                    let start = self.offset;
                    while self.offset < self.source.len() as u32 {
                        let c = self.current_char();
                        if c.is_numeric()
                            || c == '.'
                            || c == 'e'
                            || c == 'E'
                            || c == 'x'
                            || c == 'X'
                        {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token {
                        kind: TokenKind::Number,
                        offset: start,
                        length: self.offset - start,
                        line: self.line,
                        column: self.column,
                    });
                }
                _ => {
                    let punctuators = [
                        "==", "!=", "<=", ">=", "&&", "||", "<<", ">>", "->", "++", "--", "+=",
                        "-=", "*=", "/=", "++", "--",
                    ];
                    let mut matched = false;
                    for punc in punctuators {
                        if self.source[self.offset as usize..].starts_with(punc) {
                            tokens.push(Token {
                                kind: TokenKind::Punctuator,
                                offset: self.offset,
                                length: punc.len() as u32,
                                line: self.line,
                                column: self.column,
                            });
                            for _ in 0..punc.len() {
                                self.advance();
                            }
                            matched = true;
                            break;
                        }
                    }
                    if !matched {
                        let ch = self.advance();
                        if !ch.is_whitespace() {
                            tokens.push(Token {
                                kind: TokenKind::Punctuator,
                                offset: self.offset - 1,
                                length: 1,
                                line: self.line,
                                column: self.column - 1,
                            });
                        }
                    }
                }
            }
        }

        tokens.push(Token {
            kind: TokenKind::EndOfFile,
            offset: self.offset,
            length: 0,
            line: self.line,
            column: self.column,
        });

        tokens
    }

    fn current_char(&self) -> char {
        self.source
            .chars()
            .nth(self.offset as usize)
            .unwrap_or('\0')
    }

    fn peek_char(&self) -> Option<char> {
        self.source.chars().nth(self.offset as usize + 1)
    }

    fn advance(&mut self) -> char {
        let ch = self.current_char();
        self.offset += 1;
        self.column += 1;
        ch
    }

    fn newline(&mut self) {
        self.offset += 1;
        self.line += 1;
        self.column = 1;
    }

    fn make_token(&self, kind: TokenKind) -> Token {
        Token {
            kind,
            offset: self.offset,
            length: 1,
            line: self.line,
            column: self.column,
        }
    }
}

impl Default for NodeOffset {
    fn default() -> Self {
        NodeOffset::NULL
    }
}

pub mod arena {
    pub use crate::arena::{Arena, CAstNode, NodeFlags, NodeOffset};
}
