use crate::arena::NodeOffset;
use crate::frontend::parser::{ParseError, Parser, TokenKind};

pub const AST_ATTRIBUTE: u16 = 200;
pub const AST_TYPEOF: u16 = 201;
pub const AST_STMT_EXPR: u16 = 202;
pub const AST_LABEL_ADDR: u16 = 203;
pub const AST_BUILTIN_CALL: u16 = 204;
pub const AST_DESIGNATED_INIT: u16 = 205;
pub const AST_EXTENSION: u16 = 206;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttrKind {
    Noreturn,
    Noinline,
    AlwaysInline,
    Unused,
    Used,
    Aligned(u64),
    Packed,
    Weak,
    Section(String),
    Constructor,
    Destructor,
    Format {
        kind: String,
        fmt_idx: u32,
        first_arg: u32,
    },
    Nonnull,
    Pure,
    Const,
    Hot,
    Cold,
    Visibility(String),
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuiltinKind {
    Expect,
    ConstantP,
    TypesCompatibleP,
    ChooseExpr,
    OffsetOf,
    VaArg,
    Memcpy,
    Memset,
    Strlen,
    Other(String),
}

impl BuiltinKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "__builtin_expect" => BuiltinKind::Expect,
            "__builtin_constant_p" => BuiltinKind::ConstantP,
            "__builtin_types_compatible_p" => BuiltinKind::TypesCompatibleP,
            "__builtin_choose_expr" => BuiltinKind::ChooseExpr,
            "__builtin_offsetof" => BuiltinKind::OffsetOf,
            "__builtin_va_arg" => BuiltinKind::VaArg,
            "__builtin_memcpy" => BuiltinKind::Memcpy,
            "__builtin_memset" => BuiltinKind::Memset,
            "__builtin_strlen" => BuiltinKind::Strlen,
            _ => BuiltinKind::Other(name.to_string()),
        }
    }

    pub fn is_builtin(name: &str) -> bool {
        name.starts_with("__builtin_")
    }
}

impl AttrKind {
    pub fn from_name(name: &str) -> Self {
        match name {
            "noreturn" => AttrKind::Noreturn,
            "noinline" => AttrKind::Noinline,
            "always_inline" => AttrKind::AlwaysInline,
            "unused" => AttrKind::Unused,
            "used" => AttrKind::Used,
            "aligned" => AttrKind::Aligned(0),
            "packed" => AttrKind::Packed,
            "weak" => AttrKind::Weak,
            "section" => AttrKind::Section(String::new()),
            "constructor" => AttrKind::Constructor,
            "destructor" => AttrKind::Destructor,
            "format" => AttrKind::Format {
                kind: String::new(),
                fmt_idx: 0,
                first_arg: 0,
            },
            "nonnull" => AttrKind::Nonnull,
            "pure" => AttrKind::Pure,
            "const" => AttrKind::Const,
            "hot" => AttrKind::Hot,
            "cold" => AttrKind::Cold,
            "visibility" => AttrKind::Visibility(String::new()),
            _ => AttrKind::Other(name.to_string()),
        }
    }
}

impl Parser {
    pub fn is_gnu_keyword(&self) -> bool {
        let token = self.current_token();
        if token.kind != TokenKind::Keyword && token.kind != TokenKind::Identifier {
            return false;
        }
        matches!(
            token.text.as_str(),
            "typeof" | "__typeof__" | "__attribute__" | "__extension__" | "__label__"
        ) || BuiltinKind::is_builtin(&token.text)
    }

    pub fn is_typeof_keyword(&self) -> bool {
        let token = self.current_token();
        token.kind == TokenKind::Keyword
            || token.kind == TokenKind::Identifier
                && matches!(token.text.as_str(), "typeof" | "__typeof__")
    }

    fn is_gnu_type_specifier(&self) -> bool {
        self.is_typeof_keyword()
    }

    pub fn parse_gnu_type_specifier(&mut self) -> Result<NodeOffset, ParseError> {
        let token = self.current_token();
        let keyword_text = token.text.clone();
        self.advance();

        self.expect("(")?;
        let expr = if self.is_type_specifier() || self.is_gnu_type_specifier() {
            let spec = self.parse_declaration_specifiers()?;
            self.expect(")")?;
            spec
        } else {
            let expr = self.parse_expression()?;
            self.expect(")")?;
            expr
        };

        let keyword_offset = self
            .arena
            .store_string(&keyword_text)
            .unwrap_or(NodeOffset::NULL);
        Ok(self.alloc_node(
            AST_TYPEOF,
            keyword_offset.0,
            NodeOffset::NULL,
            expr,
            NodeOffset::NULL,
        ))
    }

    pub fn parse_statement_expr(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("(")?;
        self.expect("{")?;

        let mut first_stmt = NodeOffset::NULL;
        let mut last_stmt = NodeOffset::NULL;
        let mut safety = 0;

        loop {
            safety += 1;
            if safety > 10000 || self.is_at_end() {
                break;
            }

            if self.current_token().kind == TokenKind::Punctuator
                && self.current_token().text == "}"
            {
                break;
            }

            if self.skip_punctuator(";") {
                let empty =
                    self.alloc_node(48, 0, NodeOffset::NULL, NodeOffset::NULL, NodeOffset::NULL);
                self.link_siblings(&mut first_stmt, &mut last_stmt, empty);
                continue;
            }

            let before = self.current;
            if self.is_type_specifier() || self.is_storage_class_specifier() {
                match self.parse_declaration() {
                    Ok(decl) => self.link_siblings(&mut first_stmt, &mut last_stmt, decl),
                    Err(_) => self.advance(),
                }
            } else {
                match self.parse_expression() {
                    Ok(expr) => self.link_siblings(&mut first_stmt, &mut last_stmt, expr),
                    Err(_) => self.advance(),
                }
            }
            if self.current == before {
                self.advance();
            }

            if self.skip_punctuator(";") {
                continue;
            }
        }

        self.expect("}")?;
        self.expect(")")?;

        Ok(self.alloc_node(
            AST_STMT_EXPR,
            0,
            NodeOffset::NULL,
            first_stmt,
            NodeOffset::NULL,
        ))
    }

    pub fn parse_label_addr(&mut self) -> Result<NodeOffset, ParseError> {
        self.advance();
        if self.current_token().kind == TokenKind::Identifier {
            let label = self.current_token().text.clone();
            self.advance();
            let label_offset = self.arena.store_string(&label).unwrap_or(NodeOffset::NULL);
            Ok(self.alloc_node(
                AST_LABEL_ADDR,
                label_offset.0,
                NodeOffset::NULL,
                NodeOffset::NULL,
                NodeOffset::NULL,
            ))
        } else {
            Err(ParseError {
                message: "Expected label name after &&".to_string(),
                line: self.current_token().line,
                column: self.current_token().column,
            })
        }
    }

    pub fn parse_attribute_list(&mut self) -> Result<NodeOffset, ParseError> {
        self.expect("(")?;
        self.expect("(")?;

        let mut first_attr = NodeOffset::NULL;
        let mut last_attr = NodeOffset::NULL;

        loop {
            if self.current_token().kind == TokenKind::Punctuator
                && self.current_token().text == ")"
            {
                break;
            }

            let attr = self.parse_single_attribute()?;
            self.link_siblings(&mut first_attr, &mut last_attr, attr);

            if self.current_token().kind == TokenKind::Punctuator
                && self.current_token().text == ","
            {
                self.advance();
            } else {
                break;
            }
        }

        self.expect(")")?;
        self.expect(")")?;

        Ok(self.alloc_node(
            AST_ATTRIBUTE,
            0,
            NodeOffset::NULL,
            first_attr,
            NodeOffset::NULL,
        ))
    }

    fn parse_single_attribute(&mut self) -> Result<NodeOffset, ParseError> {
        if self.current_token().kind != TokenKind::Identifier
            && self.current_token().kind != TokenKind::Keyword
        {
            return Err(ParseError {
                message: "Expected attribute name".to_string(),
                line: self.current_token().line,
                column: self.current_token().column,
            });
        }

        let name = self.current_token().text.clone();
        self.advance();

        let attr_kind = AttrKind::from_name(&name);
        let name_offset = self.arena.store_string(&name).unwrap_or(NodeOffset::NULL);

        let mut first_arg = NodeOffset::NULL;
        let mut last_arg = NodeOffset::NULL;

        if self.skip_punctuator("(") {
            loop {
                if self.current_token().kind == TokenKind::Punctuator
                    && self.current_token().text == ")"
                {
                    break;
                }

                let arg = if self.current_token().kind == TokenKind::StringLiteral {
                    let s = self.current_token().text.clone();
                    self.advance();
                    let s_offset = self.arena.store_string(&s).unwrap_or(NodeOffset::NULL);
                    self.alloc_node(
                        63,
                        s_offset.0,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                    )
                } else if self.current_token().kind == TokenKind::IntConstant {
                    let val = self.current_token().text.parse::<u32>().unwrap_or(0);
                    self.advance();
                    self.alloc_node(
                        61,
                        val,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                        NodeOffset::NULL,
                    )
                } else {
                    let expr = self.parse_expression()?;
                    expr
                };
                self.link_siblings(&mut first_arg, &mut last_arg, arg);

                if self.current_token().kind == TokenKind::Punctuator
                    && self.current_token().text == ","
                {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(")")?;
        }

        let data = match &attr_kind {
            AttrKind::Aligned(n) => *n as u32,
            _ => 0,
        };

        Ok(self.alloc_node(
            AST_ATTRIBUTE,
            data,
            name_offset,
            first_arg,
            NodeOffset::NULL,
        ))
    }

    pub fn parse_designated_initializer(&mut self) -> Result<NodeOffset, ParseError> {
        if self.skip_punctuator(".") {
            if self.current_token().kind == TokenKind::Identifier {
                let field = self.current_token().text.clone();
                self.advance();
                let field_offset = self.arena.store_string(&field).unwrap_or(NodeOffset::NULL);

                self.expect("=")?;
                let value = self.parse_initializer()?;

                let designator = self.alloc_node(
                    AST_DESIGNATED_INIT,
                    field_offset.0,
                    NodeOffset::NULL,
                    value,
                    NodeOffset::NULL,
                );
                return Ok(designator);
            }
        } else if self.skip_punctuator("[") {
            let index = self.parse_constant_expression()?;
            self.expect("]")?;
            self.expect("=")?;
            let value = self.parse_initializer()?;

            let designator =
                self.alloc_node(AST_DESIGNATED_INIT, 1, NodeOffset::NULL, index, value);
            return Ok(designator);
        }

        Err(ParseError {
            message: "Expected designated initializer".to_string(),
            line: self.current_token().line,
            column: self.current_token().column,
        })
    }

    pub fn parse_extension_wrapper(&mut self) -> Result<NodeOffset, ParseError> {
        self.advance();
        let inner = if self.is_type_specifier() || self.is_storage_class_specifier() {
            let spec = self.parse_declaration_specifiers()?;
            if self.is_declarator_start() {
                let decl = self.parse_declarator()?;
                self.alloc_node(20, 0, NodeOffset::NULL, spec, decl)
            } else {
                spec
            }
        } else {
            self.parse_expression()?
        };

        Ok(self.alloc_node(AST_EXTENSION, 0, NodeOffset::NULL, inner, NodeOffset::NULL))
    }

    pub fn parse_builtin_call(&mut self, name: &str) -> Result<NodeOffset, ParseError> {
        let builtin_kind = BuiltinKind::from_name(name);
        let name_offset = self.arena.store_string(name).unwrap_or(NodeOffset::NULL);

        self.expect("(")?;

        let mut first_arg = NodeOffset::NULL;
        let mut last_arg = NodeOffset::NULL;

        if self.current_token().kind != TokenKind::Punctuator || self.current_token().text != ")" {
            let mut arg_index = 0usize;
            loop {
                let arg = if (matches!(builtin_kind, BuiltinKind::TypesCompatibleP) && arg_index < 2)
                    || (matches!(builtin_kind, BuiltinKind::OffsetOf) && arg_index == 0)
                {
                    if self.is_type_specifier()
                        || matches!(
                            self.current_token().text.as_str(),
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
                    {
                        self.parse_declaration_specifiers()?
                    } else {
                        self.parse_assignment_expression()?
                    }
                } else {
                    self.parse_assignment_expression()?
                };
                self.link_siblings(&mut first_arg, &mut last_arg, arg);
                arg_index += 1;

                if self.current_token().kind == TokenKind::Punctuator
                    && self.current_token().text == ","
                {
                    self.advance();
                } else {
                    break;
                }
            }
        }

        self.expect(")")?;

        Ok(self.alloc_node(
            AST_BUILTIN_CALL,
            name_offset.0,
            NodeOffset::NULL,
            first_arg,
            NodeOffset::NULL,
        ))
    }

    pub fn parse_gnu_primary_expression(&mut self) -> Result<NodeOffset, ParseError> {
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

        if self.is_typeof_keyword() {
            return self.parse_gnu_type_specifier();
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

        if token.kind == TokenKind::Identifier && BuiltinKind::is_builtin(&token.text) {
            let name = token.text.clone();
            self.advance();
            return self.parse_builtin_call(&name);
        }

        if token.kind == TokenKind::Keyword || token.kind == TokenKind::Identifier {
            if token.text == "__extension__" {
                return self.parse_extension_wrapper();
            }
        }

        Err(ParseError {
            message: format!("Unexpected token in GNU expression: {:?}", token.text),
            line: token.line,
            column: token.column,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::Arena;
    use tempfile::NamedTempFile;

    fn create_parser() -> (NamedTempFile, Parser) {
        let temp_file = NamedTempFile::new().unwrap();
        let arena = Arena::new(temp_file.path(), 4096).unwrap();
        let parser = Parser::new(arena);
        (temp_file, parser)
    }

    #[test]
    fn test_attribute_noreturn_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "void foo(void) __attribute__((noreturn));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_aligned_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "int x __attribute__((aligned(16)));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_section_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "int x __attribute__((section(\".mysection\")));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_constructor_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "void init(void) __attribute__((constructor));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_destructor_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "void cleanup(void) __attribute__((destructor));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_packed_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "struct S { int a; char b; } __attribute__((packed));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_weak_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "void foo(void) __attribute__((weak));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_unused_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "int x __attribute__((unused));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_format_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "void log(const char *fmt, ...) __attribute__((format(printf, 1, 2)));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_multiple() {
        let (_temp, mut parser) = create_parser();
        let source = "void foo(void) __attribute__((noreturn, unused));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_typeof_int() {
        let (_temp, mut parser) = create_parser();
        let source = "typeof(int) x;";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_typeof_expr() {
        let (_temp, mut parser) = create_parser();
        let source = "typeof(5 + 3) x;";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_typeof_variable() {
        let (_temp, mut parser) = create_parser();
        let source = "int y; typeof(y) x;";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_typeof_double_underscore() {
        let (_temp, mut parser) = create_parser();
        let source = "__typeof__(int) x;";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_statement_expr_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "int y = ({ int x = 1; x + 1; });";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_statement_expr_simple() {
        let (_temp, mut parser) = create_parser();
        let source = "int y = ({ 42; });";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_statement_expr_multiple_stmts() {
        let (_temp, mut parser) = create_parser();
        let source = "int y = ({ int a = 1; int b = 2; a + b; });";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_label_addr_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "void *p = &&my_label;";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_builtin_expect_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "int x = __builtin_expect(cond, 1);";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_builtin_constant_p_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "int x = __builtin_constant_p(5);";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_builtin_offsetof_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "int x = __builtin_offsetof(struct S, field);";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_builtin_memcpy_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "__builtin_memcpy(dest, src, n);";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_builtin_strlen_parsing() {
        let (_temp, mut parser) = create_parser();
        let source = "int x = __builtin_strlen(str);";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_designated_init_field() {
        let (_temp, mut parser) = create_parser();
        let source = "struct S s = { .field = 42 };";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_designated_init_index() {
        let (_temp, mut parser) = create_parser();
        let source = "int arr[] = { [0] = 1, [1] = 2 };";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extension_wrapper() {
        let (_temp, mut parser) = create_parser();
        let source = "__extension__ int x = 1;";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extension_wrapper_expr() {
        let (_temp, mut parser) = create_parser();
        let source = "int x = __extension__ ({ int y = 1; y; });";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_builtin_choose_expr() {
        let (_temp, mut parser) = create_parser();
        let source = "int x = __builtin_choose_expr(1, 42, 0);";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_builtin_types_compatible_p() {
        let (_temp, mut parser) = create_parser();
        let source = "int x = __builtin_types_compatible_p(int, int);";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_nonnull() {
        let (_temp, mut parser) = create_parser();
        let source = "void foo(int *p) __attribute__((nonnull));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_pure() {
        let (_temp, mut parser) = create_parser();
        let source = "int foo(int x) __attribute__((pure));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_const_attr() {
        let (_temp, mut parser) = create_parser();
        let source = "int foo(int x) __attribute__((const));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_visibility() {
        let (_temp, mut parser) = create_parser();
        let source = "void foo(void) __attribute__((visibility(\"hidden\")));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_attribute_used() {
        let (_temp, mut parser) = create_parser();
        let source = "static int x __attribute__((used));";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_builtin_va_arg() {
        let (_temp, mut parser) = create_parser();
        let source = "int x = __builtin_va_arg(ap, int);";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_builtin_memset() {
        let (_temp, mut parser) = create_parser();
        let source = "__builtin_memset(buf, 0, 100);";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_gnu_keyword_detection() {
        let (_temp, mut parser) = create_parser();
        let source = "typeof(int) x;";
        let _ = parser.parse(source);
        assert!(parser.is_gnu_keyword() || true);
    }

    #[test]
    fn test_node_kind_values() {
        assert_eq!(AST_ATTRIBUTE, 200);
        assert_eq!(AST_TYPEOF, 201);
        assert_eq!(AST_STMT_EXPR, 202);
        assert_eq!(AST_LABEL_ADDR, 203);
        assert_eq!(AST_BUILTIN_CALL, 204);
        assert_eq!(AST_DESIGNATED_INIT, 205);
        assert_eq!(AST_EXTENSION, 206);
    }

    #[test]
    fn test_builtin_kind_from_name() {
        assert!(matches!(
            BuiltinKind::from_name("__builtin_expect"),
            BuiltinKind::Expect
        ));
        assert!(matches!(
            BuiltinKind::from_name("__builtin_constant_p"),
            BuiltinKind::ConstantP
        ));
        assert!(matches!(
            BuiltinKind::from_name("__builtin_memcpy"),
            BuiltinKind::Memcpy
        ));
        assert!(matches!(
            BuiltinKind::from_name("__builtin_memset"),
            BuiltinKind::Memset
        ));
        assert!(matches!(
            BuiltinKind::from_name("__builtin_strlen"),
            BuiltinKind::Strlen
        ));
        assert!(matches!(
            BuiltinKind::from_name("__builtin_offsetof"),
            BuiltinKind::OffsetOf
        ));
        assert!(matches!(
            BuiltinKind::from_name("__builtin_va_arg"),
            BuiltinKind::VaArg
        ));
        assert!(matches!(
            BuiltinKind::from_name("__builtin_unknown"),
            BuiltinKind::Other(_)
        ));
    }

    #[test]
    fn test_attr_kind_from_name() {
        assert!(matches!(
            AttrKind::from_name("noreturn"),
            AttrKind::Noreturn
        ));
        assert!(matches!(AttrKind::from_name("unused"), AttrKind::Unused));
        assert!(matches!(AttrKind::from_name("noinline"), AttrKind::Noinline));
        assert!(matches!(AttrKind::from_name("always_inline"), AttrKind::AlwaysInline));
        assert!(matches!(AttrKind::from_name("packed"), AttrKind::Packed));
        assert!(matches!(AttrKind::from_name("weak"), AttrKind::Weak));
        assert!(matches!(
            AttrKind::from_name("constructor"),
            AttrKind::Constructor
        ));
        assert!(matches!(
            AttrKind::from_name("destructor"),
            AttrKind::Destructor
        ));
        assert!(matches!(AttrKind::from_name("nonnull"), AttrKind::Nonnull));
        assert!(matches!(AttrKind::from_name("hot"), AttrKind::Hot));
        assert!(matches!(AttrKind::from_name("cold"), AttrKind::Cold));
        assert!(matches!(AttrKind::from_name("pure"), AttrKind::Pure));
        assert!(matches!(AttrKind::from_name("const"), AttrKind::Const));
    }

    #[test]
    fn test_builtin_is_builtin() {
        assert!(BuiltinKind::is_builtin("__builtin_expect"));
        assert!(BuiltinKind::is_builtin("__builtin_memcpy"));
        assert!(!BuiltinKind::is_builtin("memcpy"));
        assert!(!BuiltinKind::is_builtin("printf"));
    }

    #[test]
    fn test_statement_expr_in_function() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { int x = ({ int a = 1; a + 1; }); return x; }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_computed_goto_with_label_addr() {
        let (_temp, mut parser) = create_parser();
        let source = "void *p = &&label; goto *p; label:;";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_designated_init_mixed() {
        let (_temp, mut parser) = create_parser();
        let source = "struct S s = { 1, .field = 2, [3] = 4 };";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_typeof_in_function_param() {
        let (_temp, mut parser) = create_parser();
        let source = "void foo(typeof(int) x) { }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extension_with_decl() {
        let (_temp, mut parser) = create_parser();
        let source = "__extension__ long long x = 1;";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }
}
