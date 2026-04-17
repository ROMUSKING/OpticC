use crate::db::OpticDb;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub line: u32,
    pub column: u32,
    pub file: String,
}

impl Token {
    pub fn new(kind: TokenKind, text: String, line: u32, column: u32, file: String) -> Self {
        Self {
            kind,
            text,
            line,
            column,
            file,
        }
    }

    pub fn is_whitespace(&self) -> bool {
        self.kind == TokenKind::Whitespace
    }

    pub fn is_newline(&self) -> bool {
        self.kind == TokenKind::Whitespace && self.text.contains('\n')
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Identifier,
    Keyword,
    IntLiteral,
    FloatLiteral,
    CharLiteral,
    StringLiteral,
    Punctuator,
    Preprocessor,
    Whitespace,
    Comment,
    EndOfFile,
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TokenKind::Identifier => write!(f, "identifier"),
            TokenKind::Keyword => write!(f, "keyword"),
            TokenKind::IntLiteral => write!(f, "int literal"),
            TokenKind::FloatLiteral => write!(f, "float literal"),
            TokenKind::CharLiteral => write!(f, "char literal"),
            TokenKind::StringLiteral => write!(f, "string literal"),
            TokenKind::Punctuator => write!(f, "punctuator"),
            TokenKind::Preprocessor => write!(f, "preprocessor"),
            TokenKind::Whitespace => write!(f, "whitespace"),
            TokenKind::Comment => write!(f, "comment"),
            TokenKind::EndOfFile => write!(f, "end of file"),
        }
    }
}

#[derive(Debug)]
pub enum PreprocessorError {
    FileNotFound(String),
    IncludeError(String, String),
    MacroError(String),
    ConditionalError(String),
    DbError(String),
    IoError(std::io::Error),
}

impl fmt::Display for PreprocessorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreprocessorError::FileNotFound(path) => write!(f, "file not found: {}", path),
            PreprocessorError::IncludeError(file, reason) => {
                write!(f, "include error in {}: {}", file, reason)
            }
            PreprocessorError::MacroError(msg) => write!(f, "macro error: {}", msg),
            PreprocessorError::ConditionalError(msg) => write!(f, "conditional error: {}", msg),
            PreprocessorError::DbError(msg) => write!(f, "database error: {}", msg),
            PreprocessorError::IoError(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for PreprocessorError {}

impl From<std::io::Error> for PreprocessorError {
    fn from(e: std::io::Error) -> Self {
        PreprocessorError::IoError(e)
    }
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

#[derive(Debug, Clone)]
struct IncludeGuard {
    ifndef_name: String,
    define_name: String,
    endif_line: u32,
}

#[derive(Clone)]
struct PpToken {
    kind: PpTokenKind,
    text: String,
    line: u32,
    column: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PpTokenKind {
    Identifier,
    Number,
    StringLit,
    CharLit,
    Punctuator,
    Whitespace,
    Newline,
    Hash,
    HashHash,
}

pub struct Preprocessor {
    db: OpticDb,
    include_paths: Vec<PathBuf>,
    macros: HashMap<String, MacroDefinition>,
    pragmas: Vec<String>,
    warnings: Vec<String>,
    errors: Vec<String>,
    current_file: String,
    include_stack: Vec<String>,
    include_guards: HashMap<String, bool>,
    active_include_guards: Vec<IncludeGuard>,
}

impl Preprocessor {
    pub fn new(db: OpticDb) -> Self {
        let mut p = Self {
            db,
            include_paths: Vec::new(),
            macros: HashMap::new(),
            pragmas: Vec::new(),
            warnings: Vec::new(),
            errors: Vec::new(),
            current_file: String::new(),
            include_stack: Vec::new(),
            include_guards: HashMap::new(),
            active_include_guards: Vec::new(),
        };
        p.define_predefined_macros();
        p
    }

    pub fn add_include_path(&mut self, path: &str) {
        self.include_paths.push(PathBuf::from(path));
    }

    pub fn define_macro(&mut self, name: &str, value: &str) {
        let tokens = self.tokenize_replacement(value);
        self.macros.insert(
            name.to_string(),
            MacroDefinition::ObjectLike {
                replacement: tokens,
            },
        );
    }

    pub fn process(&mut self, file: &str) -> Result<Vec<Token>, PreprocessorError> {
        let content = fs::read_to_string(file)
            .map_err(|e| PreprocessorError::FileNotFound(format!("{}: {}", file, e)))?;
        self.current_file = file.to_string();
        self.include_stack.push(file.to_string());
        let tokens = self.resolve_includes(&content, file, 1)?;
        self.include_stack.pop();
        Ok(tokens)
    }

    pub fn process_source(
        &mut self,
        source: &str,
        file_name: &str,
    ) -> Result<Vec<Token>, PreprocessorError> {
        self.current_file = file_name.to_string();
        self.include_stack.push(file_name.to_string());
        let tokens = self.resolve_includes(source, file_name, 1)?;
        self.include_stack.pop();
        Ok(tokens)
    }

    pub fn get_pragmas(&self) -> &[String] {
        &self.pragmas
    }

    pub fn get_warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn get_errors(&self) -> &[String] {
        &self.errors
    }

    fn define_predefined_macros(&mut self) {
        let predefined_int_macros = [
            ("__STDC__", "1".to_string()),
            ("__STDC_VERSION__", "201112L".to_string()),
            ("__STDC_HOSTED__", "1".to_string()),
            ("__GNUC__", "4".to_string()),
            ("__GNUC_MINOR__", "2".to_string()),
            ("__GNUC_PATCHLEVEL__", "1".to_string()),
            ("__SIZEOF_SHORT__", std::mem::size_of::<i16>().to_string()),
            ("__SIZEOF_INT__", std::mem::size_of::<i32>().to_string()),
            (
                "__SIZEOF_LONG__",
                std::mem::size_of::<libc::c_long>().to_string(),
            ),
            (
                "__SIZEOF_LONG_LONG__",
                std::mem::size_of::<i64>().to_string(),
            ),
            (
                "__SIZEOF_POINTER__",
                std::mem::size_of::<usize>().to_string(),
            ),
            (
                "__SIZEOF_SIZE_T__",
                std::mem::size_of::<usize>().to_string(),
            ),
            (
                "__SIZEOF_PTRDIFF_T__",
                std::mem::size_of::<isize>().to_string(),
            ),
        ];

        for (name, value) in predefined_int_macros {
            self.macros.insert(
                name.to_string(),
                MacroDefinition::ObjectLike {
                    replacement: vec![Token::new(
                        TokenKind::IntLiteral,
                        value,
                        0,
                        0,
                        String::new(),
                    )],
                },
            );
        }
    }

    fn tokenize_replacement(&self, text: &str) -> Vec<Token> {
        let mut tokens = Vec::new();
        let mut chars = text.chars().peekable();

        while let Some(&ch) = chars.peek() {
            match ch {
                ' ' | '\t' | '\r' => {
                    let mut ws = String::new();
                    while let Some(&c) = chars.peek() {
                        if c == ' ' || c == '\t' || c == '\r' {
                            ws.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    tokens.push(Token::new(TokenKind::Whitespace, ws, 0, 0, String::new()));
                }
                '\n' => {
                    chars.next();
                }
                '"' => {
                    chars.next();
                    let mut s = String::from('"');
                    while let Some(&c) = chars.peek() {
                        s.push(c);
                        chars.next();
                        if c == '"' {
                            break;
                        }
                        if c == '\\' {
                            if let Some(&nc) = chars.peek() {
                                s.push(nc);
                                chars.next();
                            }
                        }
                    }
                    tokens.push(Token::new(TokenKind::StringLiteral, s, 0, 0, String::new()));
                }
                '\'' => {
                    chars.next();
                    let mut s = String::from('\'');
                    while let Some(&c) = chars.peek() {
                        s.push(c);
                        chars.next();
                        if c == '\'' {
                            break;
                        }
                        if c == '\\' {
                            if let Some(&nc) = chars.peek() {
                                s.push(nc);
                                chars.next();
                            }
                        }
                    }
                    tokens.push(Token::new(TokenKind::CharLiteral, s, 0, 0, String::new()));
                }
                '0'..='9' => {
                    let mut num = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_alphanumeric() || c == '.' || c == 'x' || c == 'X' {
                            num.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    if num.contains('.') || num.contains('e') || num.contains('E') {
                        tokens.push(Token::new(
                            TokenKind::FloatLiteral,
                            num,
                            0,
                            0,
                            String::new(),
                        ));
                    } else {
                        tokens.push(Token::new(TokenKind::IntLiteral, num, 0, 0, String::new()));
                    }
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    let mut id = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_alphanumeric() || c == '_' {
                            id.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    let kind = if Self::is_keyword(&id) {
                        TokenKind::Keyword
                    } else {
                        TokenKind::Identifier
                    };
                    tokens.push(Token::new(kind, id, 0, 0, String::new()));
                }
                '#' => {
                    chars.next();
                    if chars.peek() == Some(&'#') {
                        chars.next();
                        tokens.push(Token::new(
                            TokenKind::Punctuator,
                            "##".to_string(),
                            0,
                            0,
                            String::new(),
                        ));
                    } else {
                        tokens.push(Token::new(
                            TokenKind::Punctuator,
                            "#".to_string(),
                            0,
                            0,
                            String::new(),
                        ));
                    }
                }
                _ => {
                    chars.next();
                    tokens.push(Token::new(
                        TokenKind::Punctuator,
                        ch.to_string(),
                        0,
                        0,
                        String::new(),
                    ));
                }
            }
        }
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

    fn tokenize_pp_source(source: &str, file: &str, base_line: u32) -> Vec<PpToken> {
        let mut tokens = Vec::new();
        let mut chars = source.chars().peekable();
        let mut line = base_line;
        let mut col: u32 = 1;

        while let Some(&ch) = chars.peek() {
            match ch {
                '\n' => {
                    chars.next();
                    tokens.push(PpToken {
                        kind: PpTokenKind::Newline,
                        text: "\n".to_string(),
                        line,
                        column: col,
                    });
                    line += 1;
                    col = 1;
                }
                ' ' | '\t' | '\r' => {
                    let mut ws = String::new();
                    while let Some(&c) = chars.peek() {
                        if c == ' ' || c == '\t' || c == '\r' {
                            ws.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                    col += ws.len() as u32;
                    tokens.push(PpToken {
                        kind: PpTokenKind::Whitespace,
                        text: ws,
                        line,
                        column: col,
                    });
                }
                '"' => {
                    let start_col = col;
                    chars.next();
                    col += 1;
                    let mut s = String::from('"');
                    while let Some(&c) = chars.peek() {
                        s.push(c);
                        chars.next();
                        col += 1;
                        if c == '"' {
                            break;
                        }
                        if c == '\\' {
                            if let Some(&nc) = chars.peek() {
                                s.push(nc);
                                chars.next();
                                col += 1;
                            }
                        }
                    }
                    tokens.push(PpToken {
                        kind: PpTokenKind::StringLit,
                        text: s,
                        line,
                        column: start_col,
                    });
                }
                '\'' => {
                    let start_col = col;
                    chars.next();
                    col += 1;
                    let mut s = String::from('\'');
                    while let Some(&c) = chars.peek() {
                        s.push(c);
                        chars.next();
                        col += 1;
                        if c == '\'' {
                            break;
                        }
                        if c == '\\' {
                            if let Some(&nc) = chars.peek() {
                                s.push(nc);
                                chars.next();
                                col += 1;
                            }
                        }
                    }
                    tokens.push(PpToken {
                        kind: PpTokenKind::CharLit,
                        text: s,
                        line,
                        column: start_col,
                    });
                }
                '/' if chars.clone().nth(1) == Some('/') => {
                    let start_col = col;
                    let mut comment = String::new();
                    while let Some(&c) = chars.peek() {
                        comment.push(c);
                        chars.next();
                        col += 1;
                        if c == '\n' {
                            break;
                        }
                    }
                    tokens.push(PpToken {
                        kind: PpTokenKind::Whitespace,
                        text: comment,
                        line,
                        column: start_col,
                    });
                }
                '/' if chars.clone().nth(1) == Some('*') => {
                    let start_col = col;
                    let mut comment = String::new();
                    comment.push(chars.next().unwrap());
                    comment.push(chars.next().unwrap());
                    col += 2;
                    while let Some(&c) = chars.peek() {
                        comment.push(c);
                        chars.next();
                        col += 1;
                        if c == '*' && chars.peek() == Some(&'/') {
                            comment.push(chars.next().unwrap());
                            col += 1;
                            break;
                        }
                        if c == '\n' {
                            line += 1;
                            col = 1;
                        }
                    }
                    tokens.push(PpToken {
                        kind: PpTokenKind::Whitespace,
                        text: comment,
                        line,
                        column: start_col,
                    });
                }
                '0'..='9' => {
                    let start_col = col;
                    let mut num = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_alphanumeric() || c == '.' || c == 'x' || c == 'X' {
                            num.push(chars.next().unwrap());
                            col += 1;
                        } else {
                            break;
                        }
                    }
                    tokens.push(PpToken {
                        kind: PpTokenKind::Number,
                        text: num,
                        line,
                        column: start_col,
                    });
                }
                'a'..='z' | 'A'..='Z' | '_' => {
                    let start_col = col;
                    let mut id = String::new();
                    while let Some(&c) = chars.peek() {
                        if c.is_alphanumeric() || c == '_' {
                            id.push(chars.next().unwrap());
                            col += 1;
                        } else {
                            break;
                        }
                    }
                    tokens.push(PpToken {
                        kind: PpTokenKind::Identifier,
                        text: id,
                        line,
                        column: start_col,
                    });
                }
                '#' => {
                    let start_col = col;
                    chars.next();
                    col += 1;
                    if chars.peek() == Some(&'#') {
                        chars.next();
                        col += 1;
                        tokens.push(PpToken {
                            kind: PpTokenKind::HashHash,
                            text: "##".to_string(),
                            line,
                            column: start_col,
                        });
                    } else {
                        tokens.push(PpToken {
                            kind: PpTokenKind::Hash,
                            text: "#".to_string(),
                            line,
                            column: start_col,
                        });
                    }
                }
                _ => {
                    let start_col = col;
                    let ch = chars.next().unwrap();
                    col += 1;
                    let mut text = ch.to_string();
                    if let Some(&next) = chars.peek() {
                        let two_char = format!("{}{}", ch, next);
                        if matches!(
                            two_char.as_str(),
                            "==" | "!=" | "<=" | ">=" | "&&" | "||" | "<<" | ">>"
                        ) {
                            chars.next();
                            col += 1;
                            text = two_char;
                        }
                    }
                    tokens.push(PpToken {
                        kind: PpTokenKind::Punctuator,
                        text,
                        line,
                        column: start_col,
                    });
                }
            }
        }
        tokens
    }

    fn pp_tokens_to_tokens(pp_tokens: &[PpToken], file: &str) -> Vec<Token> {
        pp_tokens
            .iter()
            .filter(|t| t.kind != PpTokenKind::Newline)
            .map(|t| {
                let kind = match t.kind {
                    PpTokenKind::Identifier => {
                        if Self::is_keyword(&t.text) {
                            TokenKind::Keyword
                        } else {
                            TokenKind::Identifier
                        }
                    }
                    PpTokenKind::Number => {
                        if t.text.contains('.') || t.text.contains('e') || t.text.contains('E') {
                            TokenKind::FloatLiteral
                        } else {
                            TokenKind::IntLiteral
                        }
                    }
                    PpTokenKind::StringLit => TokenKind::StringLiteral,
                    PpTokenKind::CharLit => TokenKind::CharLiteral,
                    PpTokenKind::Punctuator | PpTokenKind::Hash | PpTokenKind::HashHash => {
                        TokenKind::Punctuator
                    }
                    PpTokenKind::Whitespace => TokenKind::Whitespace,
                    PpTokenKind::Newline => TokenKind::Whitespace,
                };
                Token::new(kind, t.text.clone(), t.line, t.column, file.to_string())
            })
            .collect()
    }

    fn resolve_includes(
        &mut self,
        source: &str,
        file: &str,
        base_line: u32,
    ) -> Result<Vec<Token>, PreprocessorError> {
        let pp_tokens = Self::tokenize_pp_source(source, file, base_line);
        let mut result = Vec::new();
        let mut pending: Vec<PpToken> = Vec::new();
        let mut i = 0;
        let mut in_if_stack: Vec<bool> = Vec::new();
        let mut branch_taken: Vec<bool> = Vec::new();

        let mut flush_pending =
            |pending: &mut Vec<PpToken>, result: &mut Vec<Token>, file: &str, pp: &Preprocessor| {
                if pending.is_empty() {
                    return;
                }
                let tokens = Self::pp_tokens_to_tokens(pending, file);
                let expanded = pp.expand_macro_tokens(&tokens, file);
                result.extend(expanded);
                pending.clear();
            };

        while i < pp_tokens.len() {
            if self.is_directive_start(&pp_tokens, i) {
                flush_pending(&mut pending, &mut result, file, self);
                let (directive, dir_line, _dir_col, after_name) =
                    self.parse_directive_name(&pp_tokens, i);
                match directive.as_str() {
                    "include" => {
                        if in_if_stack.iter().all(|&active| active) {
                            let (include_path, end_idx) =
                                self.parse_include_path(&pp_tokens, after_name, file)?;
                            i = end_idx;
                            let resolved = self.resolve_include_file(&include_path, file)?;
                            if let Some(tokens) = resolved {
                                result.extend(tokens);
                            }
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "define" => {
                        if in_if_stack.iter().all(|&active| active) {
                            let end_idx = self.process_define(&pp_tokens, after_name, file)?;
                            i = end_idx;
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "undef" => {
                        if in_if_stack.iter().all(|&active| active) {
                            flush_pending(&mut pending, &mut result, file, self);
                            let end_idx = self.process_undef(&pp_tokens, after_name)?;
                            i = end_idx;
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "ifdef" => {
                        let (name, end_idx) =
                            self.parse_simple_directive_arg(&pp_tokens, after_name)?;
                        let active = in_if_stack.iter().all(|&a| a) && self.is_macro_defined(&name);
                        in_if_stack.push(active);
                        branch_taken.push(active);
                        i = end_idx;
                    }
                    "ifndef" => {
                        let (name, end_idx) =
                            self.parse_simple_directive_arg(&pp_tokens, after_name)?;
                        let active =
                            in_if_stack.iter().all(|&a| a) && !self.is_macro_defined(&name);
                        in_if_stack.push(active);
                        branch_taken.push(active);
                        if active {
                            self.active_include_guards.push(IncludeGuard {
                                ifndef_name: name.clone(),
                                define_name: String::new(),
                                endif_line: 0,
                            });
                        }
                        i = end_idx;
                    }
                    "if" => {
                        if in_if_stack.iter().all(|&a| a) {
                            let (value, end_idx) =
                                self.parse_if_expression(&pp_tokens, after_name, file)?;
                            in_if_stack.push(value != 0);
                            branch_taken.push(value != 0);
                            i = end_idx;
                        } else {
                            in_if_stack.push(false);
                            branch_taken.push(false);
                            let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "elif" => {
                        if in_if_stack.is_empty() {
                            return Err(PreprocessorError::ConditionalError(
                                "#elif without #if".to_string(),
                            ));
                        }
                        let parent_active =
                            in_if_stack.iter().take(in_if_stack.len() - 1).all(|&a| a);
                        let last_idx = in_if_stack.len() - 1;
                        if branch_taken[last_idx] {
                            in_if_stack[last_idx] = false;
                            let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                            i = end_idx;
                        } else if parent_active {
                            let (value, end_idx) =
                                self.parse_if_expression(&pp_tokens, after_name, file)?;
                            in_if_stack[last_idx] = value != 0;
                            if value != 0 {
                                branch_taken[last_idx] = true;
                            }
                            i = end_idx;
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "else" => {
                        if in_if_stack.is_empty() {
                            return Err(PreprocessorError::ConditionalError(
                                "#else without #if".to_string(),
                            ));
                        }
                        let parent_active =
                            in_if_stack.iter().take(in_if_stack.len() - 1).all(|&a| a);
                        let last_idx = in_if_stack.len() - 1;
                        if parent_active && !branch_taken[last_idx] {
                            in_if_stack[last_idx] = true;
                            branch_taken[last_idx] = true;
                        } else {
                            in_if_stack[last_idx] = false;
                        }
                        let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                        i = end_idx;
                    }
                    "endif" => {
                        if in_if_stack.pop().is_none() {
                            return Err(PreprocessorError::ConditionalError(
                                "#endif without #if".to_string(),
                            ));
                        }
                        branch_taken.pop();
                        if let Some(guard) = self.active_include_guards.last_mut() {
                            guard.endif_line = dir_line;
                        }
                        let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                        i = end_idx;
                    }
                    "pragma" => {
                        if in_if_stack.iter().all(|&active| active) {
                            let (text, end_idx) = self.parse_pragma_text(&pp_tokens, after_name);
                            // Handle #pragma once
                            if text.trim() == "once" {
                                if let Some(guard) = self.active_include_guards.last_mut() {
                                    guard.define_name = "__PRAGMA_ONCE__".to_string();
                                }
                                self.include_guards.insert(self.current_file.clone(), true);
                            }
                            self.pragmas.push(text);
                            i = end_idx;
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "error" => {
                        if in_if_stack.iter().all(|&active| active) {
                            let (msg, end_idx) = self.parse_diagnostic_text(&pp_tokens, after_name);
                            self.errors.push(msg.clone());
                            return Err(PreprocessorError::ConditionalError(format!(
                                "#error: {}",
                                msg
                            )));
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "warning" => {
                        if in_if_stack.iter().all(|&active| active) {
                            let (msg, end_idx) = self.parse_diagnostic_text(&pp_tokens, after_name);
                            self.warnings.push(msg);
                            i = end_idx;
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "line" => {
                        let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                        i = end_idx;
                    }
                    _ => {
                        let (_, end_idx) = self.skip_to_directive_end(&pp_tokens, i);
                        i = end_idx;
                    }
                }
            } else {
                if in_if_stack.iter().all(|&active| active) {
                    pending.push(pp_tokens[i].clone());
                }
                i += 1;
            }
        }

        flush_pending(&mut pending, &mut result, file, self);

        if !in_if_stack.is_empty() {
            return Err(PreprocessorError::ConditionalError(
                "unterminated conditional directive".to_string(),
            ));
        }

        self.check_include_guards();
        Ok(result)
    }

    fn is_directive_start(&self, tokens: &[PpToken], i: usize) -> bool {
        if i >= tokens.len() {
            return false;
        }
        if tokens[i].kind != PpTokenKind::Hash {
            return false;
        }
        let mut j = i + 1;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }
        j < tokens.len() && tokens[j].kind == PpTokenKind::Identifier
    }

    fn parse_directive_name(&self, tokens: &[PpToken], i: usize) -> (String, u32, u32, usize) {
        let mut j = i + 1;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }
        if j < tokens.len() && tokens[j].kind == PpTokenKind::Identifier {
            (
                tokens[j].text.clone(),
                tokens[j].line,
                tokens[j].column,
                j + 1,
            )
        } else {
            (String::new(), tokens[i].line, tokens[i].column, j)
        }
    }

    fn skip_to_directive_end(&self, tokens: &[PpToken], i: usize) -> (usize, usize) {
        let mut j = i;
        while j < tokens.len() && tokens[j].kind != PpTokenKind::Newline {
            j += 1;
        }
        (i, j + 1)
    }

    fn parse_include_path(
        &self,
        tokens: &[PpToken],
        start: usize,
        current_file: &str,
    ) -> Result<(String, usize), PreprocessorError> {
        let mut j = start;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }
        if j >= tokens.len() {
            return Err(PreprocessorError::IncludeError(
                current_file.to_string(),
                "expected include path".to_string(),
            ));
        }

        let is_angle = tokens[j].text == "<";
        let is_quote = tokens[j].kind == PpTokenKind::StringLit;

        if is_angle {
            let mut path = String::new();
            j += 1;
            while j < tokens.len() && tokens[j].text != ">" {
                path.push_str(&tokens[j].text);
                j += 1;
            }
            if j < tokens.len() {
                j += 1;
            }
            Ok((path, j))
        } else if is_quote {
            let text = &tokens[j].text;
            let path = if text.len() >= 2 {
                text[1..text.len() - 1].to_string()
            } else {
                text.clone()
            };
            j += 1;
            Ok((path, j))
        } else {
            Err(PreprocessorError::IncludeError(
                current_file.to_string(),
                format!("expected <file> or \"file\", found {}", tokens[j].text),
            ))
        }
    }

    fn resolve_include_file(
        &mut self,
        path: &str,
        current_file: &str,
    ) -> Result<Option<Vec<Token>>, PreprocessorError> {
        if self.include_stack.contains(&path.to_string()) {
            return Ok(None);
        }

        let mut search_paths = Vec::new();
        if let Some(parent) = Path::new(current_file).parent() {
            search_paths.push(parent.to_path_buf());
        }
        search_paths.extend(self.include_paths.clone());

        for search_path in &search_paths {
            let full_path = search_path.join(path);
            if full_path.exists() {
                if let Ok(canonical) = full_path.canonicalize() {
                    let path_str = canonical.to_string_lossy().to_string();
                    if self.include_stack.contains(&path_str) {
                        return Ok(None);
                    }
                }

                let content = fs::read_to_string(&full_path).map_err(|e| {
                    PreprocessorError::IncludeError(path.to_string(), e.to_string())
                })?;

                let hash = self.compute_hash(&content);
                if self.db.contains_file_hash(&hash).unwrap_or(false) {
                    return Ok(None);
                }
                self.db.insert_file_hash(&hash, &path).unwrap_or(());

                self.current_file = path.to_string();
                self.include_stack.push(path.to_string());
                let tokens = self.resolve_includes(&content, path, 1)?;
                self.include_stack.pop();
                return Ok(Some(tokens));
            }
        }

        Err(PreprocessorError::IncludeError(
            current_file.to_string(),
            format!("file not found: {}", path),
        ))
    }

    fn compute_hash(&self, content: &str) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hasher.finalize().into()
    }

    fn process_define(
        &mut self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<usize, PreprocessorError> {
        let mut j = start;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }
        if j >= tokens.len() || tokens[j].kind != PpTokenKind::Identifier {
            return Err(PreprocessorError::MacroError(
                "expected macro name after #define".to_string(),
            ));
        }

        let name = tokens[j].text.clone();
        j += 1;

        // C standard: function-like macros require NO whitespace between name and '('
        // If there's whitespace, it's an object-like macro
        let is_function_like = j < tokens.len() && tokens[j].text == "(";

        if is_function_like {
            let (params, is_variadic, end_params) = self.parse_macro_params(tokens, j, file)?;
            j = end_params;
            while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
                j += 1;
            }

            let replacement_start = j;
            let mut replacement_end = j;
            while replacement_end < tokens.len()
                && tokens[replacement_end].kind != PpTokenKind::Newline
            {
                replacement_end += 1;
            }

            let replacement_tokens =
                Self::pp_tokens_to_tokens(&tokens[replacement_start..replacement_end], file);
            let replacement = self.expand_tokens_in_macro(&replacement_tokens, &params, file);

            self.macros.insert(
                name,
                MacroDefinition::FunctionLike {
                    params,
                    is_variadic,
                    replacement,
                },
            );
            Ok(replacement_end + 1)
        } else {
            let replacement_start = j;
            let mut replacement_end = j;
            while replacement_end < tokens.len()
                && tokens[replacement_end].kind != PpTokenKind::Newline
            {
                replacement_end += 1;
            }

            let replacement_tokens =
                Self::pp_tokens_to_tokens(&tokens[replacement_start..replacement_end], file);
            let replacement = self.expand_tokens_in_macro(&replacement_tokens, &[], file);

            self.macros
                .insert(name, MacroDefinition::ObjectLike { replacement });
            Ok(replacement_end + 1)
        }
    }

    fn parse_macro_params(
        &self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<(Vec<String>, bool, usize), PreprocessorError> {
        let mut j = start + 1;
        let mut params = Vec::new();
        let mut is_variadic = false;

        while j < tokens.len() {
            while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
                j += 1;
            }
            if j >= tokens.len() {
                break;
            }
            if tokens[j].text == ")" {
                j += 1;
                break;
            }
            if tokens[j].text == "..." {
                is_variadic = true;
                j += 1;
                while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
                    j += 1;
                }
                if j < tokens.len() && tokens[j].text == ")" {
                    j += 1;
                }
                break;
            }
            if tokens[j].kind == PpTokenKind::Identifier {
                params.push(tokens[j].text.clone());
                j += 1;
            } else {
                return Err(PreprocessorError::MacroError(format!(
                    "expected parameter name in macro definition in {}",
                    file
                )));
            }
            while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
                j += 1;
            }
            if j < tokens.len() && tokens[j].text == "," {
                j += 1;
            }
        }

        Ok((params, is_variadic, j))
    }

    fn expand_tokens_in_macro(
        &self,
        tokens: &[Token],
        params: &[String],
        _file: &str,
    ) -> Vec<Token> {
        let mut result = Vec::new();
        let mut i = 0;
        while i < tokens.len() {
            if tokens[i].text == "##" {
                result.push(tokens[i].clone());
            } else if tokens[i].text == "#" && i + 1 < tokens.len() {
                result.push(tokens[i].clone());
            } else if tokens[i].kind == TokenKind::Identifier {
                let text = &tokens[i].text;
                if text == "__LINE__" {
                    result.push(Token::new(
                        TokenKind::IntLiteral,
                        "0".to_string(),
                        tokens[i].line,
                        tokens[i].column,
                        tokens[i].file.clone(),
                    ));
                } else if text == "__FILE__" {
                    result.push(Token::new(
                        TokenKind::StringLiteral,
                        format!("\"{}\"", tokens[i].file),
                        tokens[i].line,
                        tokens[i].column,
                        tokens[i].file.clone(),
                    ));
                } else if text == "__VA_ARGS__" {
                    // __VA_ARGS__ is replaced during macro invocation with variadic args
                    result.push(tokens[i].clone());
                } else if let Some(def) = self.macros.get(text) {
                    match def {
                        MacroDefinition::ObjectLike { replacement } => {
                            result.extend(replacement.clone());
                        }
                        MacroDefinition::FunctionLike { .. } => {
                            result.push(tokens[i].clone());
                        }
                    }
                } else {
                    result.push(tokens[i].clone());
                }
            } else {
                result.push(tokens[i].clone());
            }
            i += 1;
        }
        result
    }

    fn process_undef(
        &mut self,
        tokens: &[PpToken],
        start: usize,
    ) -> Result<usize, PreprocessorError> {
        let mut j = start;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }
        if j < tokens.len() && tokens[j].kind == PpTokenKind::Identifier {
            self.macros.remove(&tokens[j].text);
            j += 1;
        }
        while j < tokens.len() && tokens[j].kind != PpTokenKind::Newline {
            j += 1;
        }
        Ok(j + 1)
    }

    fn parse_simple_directive_arg(
        &self,
        tokens: &[PpToken],
        start: usize,
    ) -> Result<(String, usize), PreprocessorError> {
        let mut j = start;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }
        if j >= tokens.len() || tokens[j].kind != PpTokenKind::Identifier {
            return Err(PreprocessorError::ConditionalError(
                "expected identifier".to_string(),
            ));
        }
        let name = tokens[j].text.clone();
        j += 1;
        while j < tokens.len() && tokens[j].kind != PpTokenKind::Newline {
            j += 1;
        }
        Ok((name, j + 1))
    }

    fn parse_if_expression(
        &self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<(i64, usize), PreprocessorError> {
        let (value, end) = self.parse_if_or(tokens, start, file)?;
        let mut j = end;
        while j < tokens.len() && tokens[j].kind != PpTokenKind::Newline {
            j += 1;
        }
        Ok((value, j + 1))
    }

    fn parse_if_or(
        &self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<(i64, usize), PreprocessorError> {
        let (mut left, mut j) = self.parse_if_and(tokens, start, file)?;
        loop {
            while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
                j += 1;
            }
            if j < tokens.len() && tokens[j].text == "||" {
                j += 1;
                let (right, next) = self.parse_if_and(tokens, j, file)?;
                if left != 0 {
                    left = 1;
                } else {
                    left = if right != 0 { 1 } else { 0 };
                }
                j = next;
            } else {
                break;
            }
        }
        Ok((left, j))
    }

    fn parse_if_and(
        &self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<(i64, usize), PreprocessorError> {
        let (mut left, mut j) = self.parse_if_equality(tokens, start, file)?;
        loop {
            while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
                j += 1;
            }
            if j < tokens.len() && tokens[j].text == "&&" {
                j += 1;
                let (right, next) = self.parse_if_equality(tokens, j, file)?;
                left = if left != 0 && right != 0 { 1 } else { 0 };
                j = next;
            } else {
                break;
            }
        }
        Ok((left, j))
    }

    fn parse_if_equality(
        &self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<(i64, usize), PreprocessorError> {
        let (left, j) = self.parse_if_comparison(tokens, start, file)?;
        let mut j = j;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }
        if j + 1 < tokens.len() {
            let op = &tokens[j].text;
            if op == "==" || op == "!=" {
                let op_str = op.clone();
                j += 1;
                let (right, next) = self.parse_if_equality(tokens, j, file)?;
                let result = if op_str == "==" {
                    if left == right {
                        1
                    } else {
                        0
                    }
                } else {
                    if left != right {
                        1
                    } else {
                        0
                    }
                };
                return Ok((result, next));
            }
        }
        Ok((left, j))
    }

    fn parse_if_comparison(
        &self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<(i64, usize), PreprocessorError> {
        let (left, mut j) = self.parse_if_additive(tokens, start, file)?;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }
        if j + 1 < tokens.len() {
            let op = &tokens[j].text;
            if op == "<" || op == ">" || op == "<=" || op == ">=" {
                let op_str = op.clone();
                j += 1;
                let (right, next) = self.parse_if_additive(tokens, j, file)?;
                let result = match op_str.as_str() {
                    "<" => {
                        if left < right {
                            1
                        } else {
                            0
                        }
                    }
                    ">" => {
                        if left > right {
                            1
                        } else {
                            0
                        }
                    }
                    "<=" => {
                        if left <= right {
                            1
                        } else {
                            0
                        }
                    }
                    ">=" => {
                        if left >= right {
                            1
                        } else {
                            0
                        }
                    }
                    _ => 0,
                };
                return Ok((result, next));
            }
        }
        Ok((left, j))
    }

    fn parse_if_additive(
        &self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<(i64, usize), PreprocessorError> {
        let (mut left, mut j) = self.parse_if_multiplicative(tokens, start, file)?;
        loop {
            while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
                j += 1;
            }
            if j < tokens.len() && (tokens[j].text == "+" || tokens[j].text == "-") {
                let op = tokens[j].text.clone();
                j += 1;
                let (right, next) = self.parse_if_multiplicative(tokens, j, file)?;
                if op == "+" {
                    left = left.wrapping_add(right);
                } else {
                    left = left.wrapping_sub(right);
                }
                j = next;
            } else {
                break;
            }
        }
        Ok((left, j))
    }

    fn parse_if_multiplicative(
        &self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<(i64, usize), PreprocessorError> {
        let (mut left, mut j) = self.parse_if_unary(tokens, start, file)?;
        loop {
            while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
                j += 1;
            }
            if j < tokens.len()
                && (tokens[j].text == "*" || tokens[j].text == "/" || tokens[j].text == "%")
            {
                let op = tokens[j].text.clone();
                j += 1;
                let (right, next) = self.parse_if_unary(tokens, j, file)?;
                match op.as_str() {
                    "*" => left = left.wrapping_mul(right),
                    "/" => {
                        if right == 0 {
                            return Err(PreprocessorError::ConditionalError(
                                "division by zero in #if expression".to_string(),
                            ));
                        }
                        left /= right;
                    }
                    "%" => {
                        if right == 0 {
                            return Err(PreprocessorError::ConditionalError(
                                "modulo by zero in #if expression".to_string(),
                            ));
                        }
                        left %= right;
                    }
                    _ => {}
                }
                j = next;
            } else {
                break;
            }
        }
        Ok((left, j))
    }

    fn parse_if_unary(
        &self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<(i64, usize), PreprocessorError> {
        let mut j = start;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }
        if j >= tokens.len() {
            return Err(PreprocessorError::ConditionalError(
                "unexpected end of #if expression".to_string(),
            ));
        }

        if tokens[j].text == "!" {
            j += 1;
            let (value, end) = self.parse_if_unary(tokens, j, file)?;
            return Ok((if value == 0 { 1 } else { 0 }, end));
        }
        if tokens[j].text == "~" {
            j += 1;
            let (value, end) = self.parse_if_unary(tokens, j, file)?;
            return Ok((!value, end));
        }
        if tokens[j].text == "-" {
            j += 1;
            let (value, end) = self.parse_if_unary(tokens, j, file)?;
            return Ok((-value, end));
        }
        if tokens[j].text == "+" {
            j += 1;
            return self.parse_if_unary(tokens, j, file);
        }

        self.parse_if_primary(tokens, j, file)
    }

    fn parse_if_primary(
        &self,
        tokens: &[PpToken],
        start: usize,
        file: &str,
    ) -> Result<(i64, usize), PreprocessorError> {
        let mut j = start;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }
        if j >= tokens.len() {
            return Err(PreprocessorError::ConditionalError(
                "unexpected end of #if expression".to_string(),
            ));
        }

        if tokens[j].text == "(" {
            j += 1;
            let (value, end) = self.parse_if_or(tokens, j, file)?;
            let mut k = end;
            while k < tokens.len() && tokens[k].kind == PpTokenKind::Whitespace {
                k += 1;
            }
            if k < tokens.len() && tokens[k].text == ")" {
                Ok((value, k + 1))
            } else {
                Err(PreprocessorError::ConditionalError(
                    "expected ) in #if expression".to_string(),
                ))
            }
        } else if tokens[j].text == "defined" {
            self.parse_defined(tokens, j, file)
        } else if tokens[j].kind == PpTokenKind::Number {
            let text = &tokens[j].text;
            let value = self.parse_int_literal(text);
            Ok((value, j + 1))
        } else if tokens[j].kind == PpTokenKind::Identifier {
            let name = &tokens[j].text;
            if let Some(def) = self.macros.get(name) {
                match def {
                    MacroDefinition::ObjectLike { replacement } => {
                        if let Some(first) = replacement.iter().find(|t| !t.is_whitespace()) {
                            match first.kind {
                                TokenKind::IntLiteral => {
                                    return Ok((self.parse_int_literal(&first.text), j + 1));
                                }
                                TokenKind::CharLiteral => {
                                    return Ok((self.parse_char_literal(&first.text), j + 1));
                                }
                                _ => {
                                    if let Ok(val) = first.text.parse::<i64>() {
                                        return Ok((val, j + 1));
                                    }
                                }
                            }
                        }
                        Ok((1, j + 1))
                    }
                    MacroDefinition::FunctionLike { .. } => Ok((1, j + 1)),
                }
            } else {
                Ok((0, j + 1))
            }
        } else if tokens[j].kind == PpTokenKind::CharLit {
            Ok((self.parse_char_literal(&tokens[j].text), j + 1))
        } else {
            Err(PreprocessorError::ConditionalError(format!(
                "unexpected token in #if expression: {}",
                tokens[j].text
            )))
        }
    }

    fn parse_defined(
        &self,
        tokens: &[PpToken],
        start: usize,
        _file: &str,
    ) -> Result<(i64, usize), PreprocessorError> {
        let mut j = start + 1;
        while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
            j += 1;
        }

        let has_paren = j < tokens.len() && tokens[j].text == "(";
        if has_paren {
            j += 1;
            while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
                j += 1;
            }
        }

        if j >= tokens.len() || tokens[j].kind != PpTokenKind::Identifier {
            return Err(PreprocessorError::ConditionalError(
                "expected identifier after defined".to_string(),
            ));
        }

        let name = tokens[j].text.clone();
        j += 1;

        if has_paren {
            while j < tokens.len() && tokens[j].kind == PpTokenKind::Whitespace {
                j += 1;
            }
            if j < tokens.len() && tokens[j].text == ")" {
                j += 1;
            }
        }

        let value = if self.is_macro_defined(&name) { 1 } else { 0 };
        Ok((value, j))
    }

    fn parse_int_literal(&self, text: &str) -> i64 {
        let text = text.trim_end_matches(|c: char| c == 'u' || c == 'U' || c == 'l' || c == 'L');

        if text.starts_with("0x") || text.starts_with("0X") {
            i64::from_str_radix(&text[2..], 16).unwrap_or(0)
        } else if text.starts_with("0") && text.len() > 1 {
            i64::from_str_radix(text, 8).unwrap_or(0)
        } else {
            text.parse::<i64>().unwrap_or(0)
        }
    }

    fn parse_char_literal(&self, text: &str) -> i64 {
        if text.len() >= 2 {
            let inner = &text[1..text.len() - 1];
            if !inner.is_empty() {
                return inner.chars().next().unwrap_or('\0') as i64;
            }
        }
        0
    }

    fn parse_pragma_text(&self, tokens: &[PpToken], start: usize) -> (String, usize) {
        let mut j = start;
        let mut text = String::new();
        let mut first = true;
        while j < tokens.len() && tokens[j].kind != PpTokenKind::Newline {
            if tokens[j].kind != PpTokenKind::Whitespace || !first {
                if !first {
                    text.push(' ');
                }
                text.push_str(&tokens[j].text);
                first = false;
            }
            j += 1;
        }
        (text.trim().to_string(), j + 1)
    }

    fn parse_diagnostic_text(&self, tokens: &[PpToken], start: usize) -> (String, usize) {
        let mut j = start;
        let mut text = String::new();
        let mut first = true;
        while j < tokens.len() && tokens[j].kind != PpTokenKind::Newline {
            if !first {
                text.push(' ');
            }
            text.push_str(&tokens[j].text);
            first = false;
            j += 1;
        }
        (text.trim().to_string(), j + 1)
    }

    fn is_macro_defined(&self, name: &str) -> bool {
        self.macros.contains_key(name)
    }

    fn check_include_guards(&mut self) {
        let guards: Vec<_> = self.active_include_guards.drain(..).collect();
        for guard in guards {
            if !guard.define_name.is_empty() && guard.ifndef_name == guard.define_name {
                self.include_guards.insert(guard.ifndef_name.clone(), true);
            }
        }
    }

    fn expand_and_evaluate(
        &mut self,
        pp_tokens: &[PpToken],
        file: &str,
    ) -> Result<Vec<Token>, PreprocessorError> {
        let mut result = Vec::new();
        let mut i = 0;
        let mut in_if_stack: Vec<bool> = Vec::new();

        while i < pp_tokens.len() {
            if self.is_directive_start(pp_tokens, i) {
                let (directive, _dir_line, _dir_col, after_name) =
                    self.parse_directive_name(pp_tokens, i);
                match directive.as_str() {
                    "define" => {
                        if in_if_stack.iter().all(|&active| active) {
                            let end_idx = self.process_define(pp_tokens, after_name, file)?;
                            i = end_idx;
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "undef" => {
                        if in_if_stack.iter().all(|&active| active) {
                            let end_idx = self.process_undef(pp_tokens, after_name)?;
                            i = end_idx;
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "ifdef" => {
                        let (name, end_idx) =
                            self.parse_simple_directive_arg(pp_tokens, after_name)?;
                        let active = in_if_stack.iter().all(|&a| a) && self.is_macro_defined(&name);
                        in_if_stack.push(active);
                        i = end_idx;
                    }
                    "ifndef" => {
                        let (name, end_idx) =
                            self.parse_simple_directive_arg(pp_tokens, after_name)?;
                        let active =
                            in_if_stack.iter().all(|&a| a) && !self.is_macro_defined(&name);
                        in_if_stack.push(active);
                        i = end_idx;
                    }
                    "if" => {
                        if in_if_stack.iter().all(|&a| a) {
                            let (value, end_idx) =
                                self.parse_if_expression(pp_tokens, after_name, file)?;
                            in_if_stack.push(value != 0);
                            i = end_idx;
                        } else {
                            in_if_stack.push(false);
                            let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "elif" => {
                        if in_if_stack.is_empty() {
                            let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                            i = end_idx;
                        } else {
                            let last_val = *in_if_stack.last().unwrap();
                            let parent_active =
                                in_if_stack.iter().take(in_if_stack.len() - 1).all(|&a| a);
                            let last_idx = in_if_stack.len() - 1;
                            if last_val {
                                in_if_stack[last_idx] = false;
                                let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                                i = end_idx;
                            } else if parent_active {
                                let (value, end_idx) =
                                    self.parse_if_expression(pp_tokens, after_name, file)?;
                                in_if_stack[last_idx] = value != 0;
                                i = end_idx;
                            } else {
                                let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                                i = end_idx;
                            }
                        }
                    }
                    "else" => {
                        if !in_if_stack.is_empty() {
                            let parent_active =
                                in_if_stack.iter().take(in_if_stack.len() - 1).all(|&a| a);
                            let any_branch_taken = in_if_stack.iter().any(|&a| a);
                            if parent_active && !any_branch_taken {
                                let last = in_if_stack.last_mut().unwrap();
                                *last = true;
                            } else if parent_active && any_branch_taken {
                                let last = in_if_stack.last_mut().unwrap();
                                *last = false;
                            }
                        }
                        let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                        i = end_idx;
                    }
                    "endif" => {
                        in_if_stack.pop();
                        let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                        i = end_idx;
                    }
                    "pragma" => {
                        if in_if_stack.iter().all(|&active| active) {
                            let (text, end_idx) = self.parse_pragma_text(pp_tokens, after_name);
                            self.pragmas.push(text);
                            i = end_idx;
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "error" => {
                        if in_if_stack.iter().all(|&active| active) {
                            let (msg, end_idx) = self.parse_diagnostic_text(pp_tokens, after_name);
                            self.errors.push(msg.clone());
                            return Err(PreprocessorError::ConditionalError(format!(
                                "#error: {}",
                                msg
                            )));
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    "warning" => {
                        if in_if_stack.iter().all(|&active| active) {
                            let (msg, end_idx) = self.parse_diagnostic_text(pp_tokens, after_name);
                            self.warnings.push(msg);
                            i = end_idx;
                        } else {
                            let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                            i = end_idx;
                        }
                    }
                    _ => {
                        let (_, end_idx) = self.skip_to_directive_end(pp_tokens, i);
                        i = end_idx;
                    }
                }
            } else {
                if in_if_stack.iter().all(|&active| active) {
                    result.push(pp_tokens[i].clone());
                }
                i += 1;
            }
        }

        let mut tokens = Self::pp_tokens_to_tokens(&result, file);
        tokens = self.expand_macro_tokens(&tokens, file);
        Ok(tokens)
    }

    fn expand_macro_tokens(&self, tokens: &[Token], file: &str) -> Vec<Token> {
        let mut result = Vec::new();
        let mut i = 0;
        let mut expanding: Vec<String> = Vec::new();

        while i < tokens.len() {
            let token = &tokens[i];

            if token.kind == TokenKind::Whitespace {
                result.push(token.clone());
                i += 1;
                continue;
            }

            if token.kind == TokenKind::Identifier {
                let name = &token.text;

                if name == "__LINE__" {
                    result.push(Token::new(
                        TokenKind::IntLiteral,
                        token.line.to_string(),
                        token.line,
                        token.column,
                        token.file.clone(),
                    ));
                    i += 1;
                    continue;
                }

                if name == "__FILE__" {
                    result.push(Token::new(
                        TokenKind::StringLiteral,
                        format!("\"{}\"", token.file),
                        token.line,
                        token.column,
                        token.file.clone(),
                    ));
                    i += 1;
                    continue;
                }

                if expanding.contains(name) {
                    result.push(token.clone());
                    i += 1;
                    continue;
                }

                if let Some(def) = self.macros.get(name) {
                    match def {
                        MacroDefinition::ObjectLike { replacement } => {
                            expanding.push(name.clone());
                            let expanded = self.expand_macro_tokens(replacement, file);
                            expanding.pop();
                            result.extend(expanded);
                        }
                        MacroDefinition::FunctionLike {
                            params,
                            is_variadic,
                            replacement,
                        } => {
                            let mut j = i + 1;
                            while j < tokens.len() && tokens[j].is_whitespace() {
                                j += 1;
                            }
                            if j < tokens.len() && tokens[j].text == "(" {
                                expanding.push(name.clone());
                                let (args, end_idx) =
                                    self.collect_macro_args(&tokens, j, params.len(), *is_variadic);
                                i = end_idx;
                                let expanded = self.expand_function_macro_tokens(
                                    replacement,
                                    params,
                                    *is_variadic,
                                    &args,
                                    file,
                                );
                                expanding.pop();
                                result.extend(expanded);
                                continue;
                            } else {
                                result.push(token.clone());
                            }
                        }
                    }
                    i += 1;
                    continue;
                }
            }

            result.push(token.clone());
            i += 1;
        }

        result
    }

    fn collect_macro_args(
        &self,
        tokens: &[Token],
        start: usize,
        expected_params: usize,
        is_variadic: bool,
    ) -> (Vec<Vec<Token>>, usize) {
        let mut args: Vec<Vec<Token>> = Vec::new();
        let mut current_arg: Vec<Token> = Vec::new();
        let mut paren_depth = 0;
        let mut j = start;

        if j < tokens.len() && tokens[j].text == "(" {
            j += 1;
        }

        while j < tokens.len() {
            let token = &tokens[j];

            if token.text == "(" {
                paren_depth += 1;
                current_arg.push(token.clone());
            } else if token.text == ")" && paren_depth == 0 {
                if !current_arg.is_empty()
                    || args.len() < expected_params
                    || (is_variadic && args.is_empty())
                {
                    args.push(current_arg);
                }
                j += 1;
                break;
            } else if token.text == "," && paren_depth == 0 {
                args.push(current_arg);
                current_arg = Vec::new();
            } else {
                current_arg.push(token.clone());
            }

            if token.text == ")" && paren_depth > 0 {
                paren_depth -= 1;
            }

            j += 1;
        }

        (args, j)
    }

    fn expand_function_macro_tokens(
        &self,
        replacement: &[Token],
        params: &[String],
        is_variadic: bool,
        args: &[Vec<Token>],
        file: &str,
    ) -> Vec<Token> {
        let mut result = Vec::new();
        let mut i = 0;

        while i < replacement.len() {
            if replacement[i].text == "##" {
                if !result.is_empty() && i + 1 < replacement.len() {
                    let left = result.pop().unwrap();
                    let right_tokens =
                        self.get_param_replacement(&replacement[i + 1], params, args, file);
                    if let Some(right) = right_tokens.first() {
                        let pasted = self.paste_tokens(&left, right);
                        result.push(pasted);
                        for t in right_tokens.iter().skip(1) {
                            result.push(t.clone());
                        }
                    }
                    i += 2;
                    continue;
                }
                i += 1;
                continue;
            }

            if replacement[i].text == "#" && i + 1 < replacement.len() {
                let next = &replacement[i + 1];
                if next.kind == TokenKind::Identifier {
                    if let Some(param_idx) = params.iter().position(|p| p == &next.text) {
                        let stringified = self.stringify_arg(&args[param_idx]);
                        result.push(Token::new(
                            TokenKind::StringLiteral,
                            stringified,
                            next.line,
                            next.column,
                            next.file.clone(),
                        ));
                        i += 2;
                        continue;
                    }
                }
            }

            if replacement[i].kind == TokenKind::Identifier {
                // Handle __VA_ARGS__ for variadic macros
                if replacement[i].text == "__VA_ARGS__" && is_variadic && !args.is_empty() {
                    // __VA_ARGS__ is replaced with all arguments beyond the named params
                    let va_start = params.len();
                    if va_start < args.len() {
                        for arg in args.iter().skip(va_start) {
                            result.extend(arg.clone());
                        }
                    }
                    i += 1;
                    continue;
                }
                if let Some(param_idx) = params.iter().position(|p| p == &replacement[i].text) {
                    if param_idx < args.len() {
                        result.extend(args[param_idx].clone());
                        i += 1;
                        continue;
                    }
                }
            }

            result.push(replacement[i].clone());
            i += 1;
        }

        result
    }

    fn get_param_replacement(
        &self,
        token: &Token,
        params: &[String],
        args: &[Vec<Token>],
        _file: &str,
    ) -> Vec<Token> {
        if token.kind == TokenKind::Identifier {
            if let Some(param_idx) = params.iter().position(|p| p == &token.text) {
                if param_idx < args.len() {
                    return args[param_idx].clone();
                }
            }
        }
        vec![token.clone()]
    }

    fn paste_tokens(&self, left: &Token, right: &Token) -> Token {
        let pasted = format!("{}{}", left.text, right.text);
        let kind = if pasted
            .chars()
            .next()
            .map(|c| c.is_alphabetic() || c == '_')
            .unwrap_or(false)
        {
            if Self::is_keyword(&pasted) {
                TokenKind::Keyword
            } else {
                TokenKind::Identifier
            }
        } else if pasted
            .chars()
            .next()
            .map(|c| c.is_numeric())
            .unwrap_or(false)
        {
            if pasted.contains('.') || pasted.contains('e') || pasted.contains('E') {
                TokenKind::FloatLiteral
            } else {
                TokenKind::IntLiteral
            }
        } else {
            TokenKind::Punctuator
        };

        Token::new(kind, pasted, left.line, left.column, left.file.clone())
    }

    fn stringify_arg(&self, tokens: &[Token]) -> String {
        let mut text = String::from('"');
        let mut first = true;
        for token in tokens {
            if token.is_whitespace() {
                if !first {
                    text.push(' ');
                }
                continue;
            }
            if !first && !text.ends_with('"') {
                text.push(' ');
            }
            for ch in token.text.chars() {
                if ch == '"' || ch == '\\' {
                    text.push('\\');
                }
                text.push(ch);
            }
            first = false;
        }
        text.push('"');
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_preprocessor() -> (Preprocessor, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.redb");
        let db = OpticDb::new(&db_path).unwrap();
        let pp = Preprocessor::new(db);
        (pp, temp_dir)
    }

    #[test]
    fn test_basic_object_macro_expansion() {
        let (mut pp, _temp_dir) = create_test_preprocessor();
        pp.define_macro("PI", "3.14159");
        pp.define_macro("MAX", "100");

        let source = "int x = PI;\nint y = MAX;";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "3.14159"));
        assert!(non_ws.iter().any(|t| t.text == "100"));
    }

    #[test]
    fn test_function_macro_expansion() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#define ADD(a, b) a + b\nint x = ADD(1, 2);";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "1"));
        assert!(non_ws.iter().any(|t| t.text == "+"));
        assert!(non_ws.iter().any(|t| t.text == "2"));
    }

    #[test]
    fn test_function_macro_stringification() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = r#"#define STR(x) #x
const char *s = STR(hello world);"#;
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text.contains("hello")));
    }

    #[test]
    fn test_function_macro_token_pasting() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#define CONCAT(a, b) a##b\nint xy = 42;";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "xy"));
    }

    #[test]
    fn test_ifdef_conditional() {
        let (mut pp, _temp_dir) = create_test_preprocessor();
        pp.define_macro("DEBUG", "1");

        let source =
            "#ifdef DEBUG\nint debug_var = 1;\n#endif\n#ifndef DEBUG\nint no_debug = 0;\n#endif";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "debug_var"));
        assert!(!non_ws.iter().any(|t| t.text == "no_debug"));
    }

    #[test]
    fn test_ifndef_conditional() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#ifndef UNDEFINED\nint should_exist = 1;\n#endif";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "should_exist"));
    }

    #[test]
    fn test_if_with_defined_operator() {
        let (mut pp, _temp_dir) = create_test_preprocessor();
        pp.define_macro("FEATURE", "1");

        let source = "#if defined(FEATURE)\nint feature_enabled = 1;\n#endif";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "feature_enabled"));
    }

    #[test]
    fn test_if_with_defined_no_parens() {
        let (mut pp, _temp_dir) = create_test_preprocessor();
        pp.define_macro("FEATURE", "1");

        let source = "#if defined FEATURE\nint feature_enabled = 1;\n#endif";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "feature_enabled"));
    }

    #[test]
    fn test_elif_chains() {
        let (mut pp, _temp_dir) = create_test_preprocessor();
        pp.define_macro("VERSION", "2");

        let source = "#if VERSION == 1\nint v1 = 1;\n#elif VERSION == 2\nint v2 = 2;\n#elif VERSION == 3\nint v3 = 3;\n#else\nint other = 0;\n#endif";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "v2"));
        assert!(!non_ws.iter().any(|t| t.text == "v1"));
        assert!(!non_ws.iter().any(|t| t.text == "v3"));
        assert!(!non_ws.iter().any(|t| t.text == "other"));
    }

    #[test]
    fn test_include_guard_detection() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#ifndef MY_HEADER_H\n#define MY_HEADER_H\nint guarded = 1;\n#endif";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "guarded"));
    }

    #[test]
    fn test_predefined_macros() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "int a = __STDC__; int b = __GNUC_PATCHLEVEL__; int c = __SIZEOF_POINTER__;";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        let pointer_size = std::mem::size_of::<usize>().to_string();
        assert!(non_ws.iter().any(|t| t.text == "1"));
        assert!(non_ws.iter().any(|t| t.text == pointer_size));
    }

    #[test]
    fn test_predefined_macros_in_if_expressions() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#if __STDC_VERSION__ >= 201112L && __GNUC_PATCHLEVEL__ >= 0\nint standards_ok = 1;\n#endif";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "standards_ok"));
    }

    #[test]
    fn test_sizeof_predefined_macros_in_if_expressions() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#if __SIZEOF_POINTER__ >= 4 && __STDC_HOSTED__ == 1\nint abi_ok = 1;\n#endif";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "abi_ok"));
    }

    #[test]
    fn test_pragma_collection() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#pragma once\n#pragma GCC optimize O3\nint x = 1;";
        let _tokens = pp.process_source(source, "test.c").unwrap();

        let pragmas = pp.get_pragmas();
        assert!(pragmas.iter().any(|p| p.contains("once")));
        assert!(pragmas.iter().any(|p| p.contains("GCC")));
    }

    #[test]
    fn test_error_diagnostic() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#error \"This is an error\"";
        let result = pp.process_source(source, "test.c");
        assert!(result.is_err());
    }

    #[test]
    fn test_warning_diagnostic() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#warning \"This is a warning\"\nint x = 1;";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let warnings = pp.get_warnings();
        assert!(warnings.iter().any(|w| w.contains("warning")));
        assert!(!tokens.is_empty());
    }

    #[test]
    fn test_nested_includes() {
        let (mut pp, temp_dir) = create_test_preprocessor();

        let inner_path = temp_dir.path().join("inner.h");
        fs::write(&inner_path, "int inner_var = 42;").unwrap();

        let outer_path = temp_dir.path().join("outer.h");
        fs::write(&outer_path, "#include \"inner.h\"\nint outer_var = 1;").unwrap();

        let main_path = temp_dir.path().join("main.c");
        fs::write(&main_path, "#include \"outer.h\"\nint main_var = 2;").unwrap();

        pp.add_include_path(temp_dir.path().to_str().unwrap());

        let tokens = pp.process(main_path.to_str().unwrap()).unwrap();
        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();

        assert!(non_ws.iter().any(|t| t.text == "inner_var"));
        assert!(non_ws.iter().any(|t| t.text == "outer_var"));
        assert!(non_ws.iter().any(|t| t.text == "main_var"));
    }

    #[test]
    fn test_include_deduplication_via_redb() {
        let (mut pp, temp_dir) = create_test_preprocessor();

        let header_path = temp_dir.path().join("dedup.h");
        fs::write(&header_path, "int dedup_var = 99;").unwrap();

        let main_path = temp_dir.path().join("main.c");
        fs::write(
            &main_path,
            "#include \"dedup.h\"\n#include \"dedup.h\"\nint main_var = 1;",
        )
        .unwrap();

        pp.add_include_path(temp_dir.path().to_str().unwrap());

        let tokens = pp.process(main_path.to_str().unwrap()).unwrap();
        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();

        let dedup_count = non_ws.iter().filter(|t| t.text == "dedup_var").count();
        assert_eq!(dedup_count, 1);
    }

    #[test]
    fn test_undef_macro() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#define TEMP 42\nint a = TEMP;\n#undef TEMP\nint b = TEMP;";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        let temp_count = non_ws.iter().filter(|t| t.text == "42").count();
        assert_eq!(temp_count, 1);
    }

    #[test]
    fn test_if_expression_arithmetic() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#if 2 + 3 == 5\nint math_works = 1;\n#endif";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(non_ws.iter().any(|t| t.text == "math_works"));
    }

    #[test]
    fn test_token_file_tracking() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "int x = 1;";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(!non_ws.is_empty());
        assert_eq!(non_ws[0].file, "test.c");
    }

    #[test]
    fn test_if_with_logical_operators() {
        let (mut pp, _temp_dir) = create_test_preprocessor();
        pp.define_macro("A", "1");
        pp.define_macro("B", "0");

        let source = "#if A && B\nint both = 1;\n#endif\n#if A || B\nint either = 1;\n#endif";
        let tokens = pp.process_source(source, "test.c").unwrap();

        let non_ws: Vec<&Token> = tokens.iter().filter(|t| !t.is_whitespace()).collect();
        assert!(!non_ws.iter().any(|t| t.text == "both"));
        assert!(non_ws.iter().any(|t| t.text == "either"));
    }

    #[test]
    fn test_include_angle_bracket_not_found() {
        let (mut pp, _temp_dir) = create_test_preprocessor();

        let source = "#include <nonexistent.h>\nint x = 1;";
        let result = pp.process_source(source, "test.c");
        assert!(result.is_err());
    }
}
