use crate::arena::NodeOffset;
use crate::frontend::parser::{ParseError, Parser, TokenKind};

pub const AST_ASM_STMT: u16 = 207;
pub const ASM_OPERAND_OUTPUT: u16 = 208;
pub const ASM_OPERAND_INPUT: u16 = 209;
pub const ASM_CLOBBER: u16 = 210;
pub const ASM_GOTO_LABEL: u16 = 211;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsmOperand {
    pub constraint: String,
    pub expr_offset: NodeOffset,
    pub is_output: bool,
    pub is_readwrite: bool,
}

#[derive(Debug, Clone)]
pub struct AsmStmt {
    pub template: String,
    pub is_volatile: bool,
    pub outputs: Vec<AsmOperand>,
    pub inputs: Vec<AsmOperand>,
    pub clobbers: Vec<String>,
    pub goto_labels: Vec<String>,
}

impl Parser {
    pub fn is_asm_keyword(&self) -> bool {
        let token = self.current_token();
        token.text == "asm" || token.text == "__asm__" || token.text == "__asm"
    }

    pub fn parse_asm_stmt(&mut self) -> Result<NodeOffset, ParseError> {
        self.advance();

        let is_volatile = if self.current_token().text == "volatile"
            || self.current_token().text == "__volatile__"
        {
            self.advance();
            true
        } else {
            false
        };

        self.expect("(")?;

        let template = if self.current_token().kind == TokenKind::StringLiteral {
            let t = self.current_token().text.clone();
            self.advance();
            t
        } else {
            return Err(ParseError {
                message: "Expected asm template string".to_string(),
                line: self.current_token().line,
                column: self.current_token().column,
            });
        };

        let (outputs, inputs, clobbers, goto_labels) = self.parse_asm_operands()?;

        self.expect(")")?;
        self.skip_punctuator(";");

        let template_offset = self
            .arena
            .store_string(&template)
            .unwrap_or(NodeOffset::NULL);

        let mut flags: u32 = 0;
        if is_volatile {
            flags |= 1;
        }
        if !goto_labels.is_empty() {
            flags |= 2;
        }

        let mut first_child = NodeOffset::NULL;
        let mut last_child = NodeOffset::NULL;

        for output in &outputs {
            let constraint_offset = self
                .arena
                .store_string(&output.constraint)
                .unwrap_or(NodeOffset::NULL);
            let operand_node = self.alloc_node(
                ASM_OPERAND_OUTPUT,
                constraint_offset.0,
                NodeOffset::NULL,
                output.expr_offset,
                NodeOffset::NULL,
            );
            self.link_siblings(&mut first_child, &mut last_child, operand_node);
        }

        for input in &inputs {
            let constraint_offset = self
                .arena
                .store_string(&input.constraint)
                .unwrap_or(NodeOffset::NULL);
            let operand_node = self.alloc_node(
                ASM_OPERAND_INPUT,
                constraint_offset.0,
                NodeOffset::NULL,
                input.expr_offset,
                NodeOffset::NULL,
            );
            self.link_siblings(&mut first_child, &mut last_child, operand_node);
        }

        for clobber in &clobbers {
            let clobber_offset = self.arena.store_string(clobber).unwrap_or(NodeOffset::NULL);
            let clobber_node = self.alloc_node(
                ASM_CLOBBER,
                clobber_offset.0,
                NodeOffset::NULL,
                NodeOffset::NULL,
                NodeOffset::NULL,
            );
            self.link_siblings(&mut first_child, &mut last_child, clobber_node);
        }

        for label in &goto_labels {
            let label_offset = self.arena.store_string(label).unwrap_or(NodeOffset::NULL);
            let label_node = self.alloc_node(
                ASM_GOTO_LABEL,
                label_offset.0,
                NodeOffset::NULL,
                NodeOffset::NULL,
                NodeOffset::NULL,
            );
            self.link_siblings(&mut first_child, &mut last_child, label_node);
        }

        let asm_node = self.alloc_node(
            AST_ASM_STMT,
            flags,
            NodeOffset::NULL,
            template_offset,
            first_child,
        );

        Ok(asm_node)
    }

    pub fn parse_asm_operands(
        &mut self,
    ) -> Result<(Vec<AsmOperand>, Vec<AsmOperand>, Vec<String>, Vec<String>), ParseError> {
        let mut outputs: Vec<AsmOperand> = Vec::new();
        let mut inputs: Vec<AsmOperand> = Vec::new();
        let mut clobbers: Vec<String> = Vec::new();
        let mut goto_labels: Vec<String> = Vec::new();

        let mut section: u8 = 0;

        loop {
            if self.current_token().kind == TokenKind::Punctuator
                && self.current_token().text == ")"
            {
                break;
            }

            if self.current_token().kind == TokenKind::Punctuator
                && self.current_token().text == ":"
            {
                self.advance();
                section += 1;

                if self.current_token().kind == TokenKind::Punctuator
                    && (self.current_token().text == ":" || self.current_token().text == ")")
                {
                    continue;
                }
            }

            match section {
                0 => {}
                1 => {
                    if self.current_token().kind == TokenKind::StringLiteral {
                        let operand = self.parse_single_asm_operand(true)?;
                        outputs.push(operand);
                    } else {
                        break;
                    }
                }
                2 => {
                    if self.current_token().kind == TokenKind::StringLiteral {
                        let operand = self.parse_single_asm_operand(false)?;
                        inputs.push(operand);
                    } else {
                        break;
                    }
                }
                3 => {
                    if self.current_token().kind == TokenKind::StringLiteral {
                        let clobber = self.current_token().text.clone();
                        self.advance();
                        clobbers.push(clobber);
                    } else {
                        break;
                    }
                }
                4 => {
                    if self.current_token().kind == TokenKind::Identifier
                        || self.current_token().kind == TokenKind::Punctuator
                            && self.current_token().text == "&&"
                    {
                        if self.current_token().text == "&&" {
                            self.advance();
                        }
                        if self.current_token().kind == TokenKind::Identifier {
                            let label = self.current_token().text.clone();
                            self.advance();
                            goto_labels.push(label);
                        }
                    } else {
                        break;
                    }
                }
                _ => break,
            }

            if self.current_token().kind == TokenKind::Punctuator
                && self.current_token().text == ","
            {
                self.advance();
            }
        }

        Ok((outputs, inputs, clobbers, goto_labels))
    }

    pub fn parse_single_asm_operand(&mut self, is_output: bool) -> Result<AsmOperand, ParseError> {
        if self.current_token().kind != TokenKind::StringLiteral {
            return Err(ParseError {
                message: "Expected constraint string in asm operand".to_string(),
                line: self.current_token().line,
                column: self.current_token().column,
            });
        }

        let constraint = self.current_token().text.clone();
        self.advance();

        self.expect("(")?;

        let expr = self.parse_assignment_expression()?;

        self.expect(")")?;

        let is_readwrite = constraint.starts_with('+');
        let clean_constraint = if is_output && !is_readwrite {
            constraint
                .strip_prefix('=')
                .unwrap_or(&constraint)
                .to_string()
        } else if is_readwrite {
            constraint
                .strip_prefix('+')
                .unwrap_or(&constraint)
                .to_string()
        } else {
            constraint.clone()
        };

        Ok(AsmOperand {
            constraint: clean_constraint,
            expr_offset: expr,
            is_output,
            is_readwrite,
        })
    }
}

pub fn build_constraints_string(
    outputs: &[AsmOperand],
    inputs: &[AsmOperand],
    clobbers: &[String],
) -> String {
    let mut parts: Vec<String> = Vec::new();

    for output in outputs {
        let prefix = if output.is_readwrite { "+" } else { "=" };
        parts.push(format!("{}{}", prefix, output.constraint));
    }

    for input in inputs {
        parts.push(input.constraint.clone());
    }

    for clobber in clobbers {
        if clobber == "memory" || clobber == "cc" {
            parts.push(format!("~{{{}}}", clobber));
        } else {
            parts.push(format!("~{{{}}}", clobber));
        }
    }

    parts.join(",")
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

    fn find_asm_node_dfs(arena: &Arena, offset: NodeOffset) -> Option<u32> {
        let mut current = offset;
        while current != NodeOffset::NULL {
            if let Some(node) = arena.get(current) {
                if node.kind == AST_ASM_STMT {
                    return Some(node.data);
                }
                if node.first_child != NodeOffset::NULL {
                    if let Some(data) = find_asm_node_dfs(arena, node.first_child) {
                        return Some(data);
                    }
                }
                current = node.next_sibling;
            } else {
                break;
            }
        }
        None
    }

    #[test]
    fn test_basic_asm_volatile() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { asm volatile(\"nop\"); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_basic_asm_no_volatile() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { asm(\"nop\"); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extended_asm_outputs() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { int out; asm(\"mov %1, %0\" : \"=r\"(out)); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extended_asm_outputs_and_inputs() {
        let (_temp, mut parser) = create_parser();
        let source =
            "int main() { int out; int in; asm(\"mov %1, %0\" : \"=r\"(out) : \"r\"(in)); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_extended_asm_with_clobbers() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { int out; int in; asm(\"mov %1, %0\" : \"=r\"(out) : \"r\"(in) : \"memory\"); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_memory_clobber() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { asm volatile(\"\" : : : \"memory\"); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_goto() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { asm goto(\"jmp %l0\" : : : : label); label:; }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_double_underscore() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { __asm__(\"nop\"); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_volatile_double_underscore() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { __asm__ __volatile__(\"nop\"); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_readwrite_operand() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { int x; asm(\"addl %1, %0\" : \"+r\"(x) : \"r\"(1)); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_multiple_outputs() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { int a; int b; asm(\"\" : \"=r\"(a), \"=r\"(b)); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_multiple_inputs() {
        let (_temp, mut parser) = create_parser();
        let source =
            "int main() { int a; int b; int c; asm(\"\" : : \"r\"(a), \"r\"(b), \"r\"(c)); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_multiple_clobbers() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { asm(\"\" : : : \"eax\", \"ebx\", \"memory\"); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_named_operands() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { int x; asm(\"\" : [result] \"=r\"(x) : [input] \"r\"(x)); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_in_function_with_return() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { asm volatile(\"nop\"); return 0; }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_cc_clobber() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { asm(\"\" : : : \"cc\"); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_empty_clobber_section() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { int x; asm(\"\" : \"=r\"(x) : : ); }";
        let result = parser.parse(source);
        assert!(result.is_ok());
    }

    #[test]
    fn test_asm_keyword_detection() {
        let (_temp, mut parser) = create_parser();
        let _ = parser.parse("int x;");
        parser.tokens = vec![crate::frontend::parser::Token {
            kind: TokenKind::Keyword,
            text: "asm".to_string(),
            line: 1,
            column: 1,
            file: String::new(),
        }];
        parser.current = 0;
        assert!(parser.is_asm_keyword());
    }

    #[test]
    fn test_asm_underscore_keyword_detection() {
        let (_temp, mut parser) = create_parser();
        let _ = parser.parse("int x;");
        parser.tokens = vec![crate::frontend::parser::Token {
            kind: TokenKind::Keyword,
            text: "__asm__".to_string(),
            line: 1,
            column: 1,
            file: String::new(),
        }];
        parser.current = 0;
        assert!(parser.is_asm_keyword());
    }

    #[test]
    fn test_build_constraints_string() {
        let outputs = vec![AsmOperand {
            constraint: "r".to_string(),
            expr_offset: NodeOffset::NULL,
            is_output: true,
            is_readwrite: false,
        }];
        let inputs = vec![AsmOperand {
            constraint: "r".to_string(),
            expr_offset: NodeOffset::NULL,
            is_output: false,
            is_readwrite: false,
        }];
        let clobbers = vec!["memory".to_string()];

        let constraints = build_constraints_string(&outputs, &inputs, &clobbers);
        assert!(constraints.contains("=r"));
        assert!(constraints.contains("~{memory}"));
    }

    #[test]
    fn test_build_constraints_readwrite() {
        let outputs = vec![AsmOperand {
            constraint: "r".to_string(),
            expr_offset: NodeOffset::NULL,
            is_output: true,
            is_readwrite: true,
        }];
        let inputs: Vec<AsmOperand> = Vec::new();
        let clobbers: Vec<String> = Vec::new();

        let constraints = build_constraints_string(&outputs, &inputs, &clobbers);
        assert!(constraints.contains("+r"));
    }

    #[test]
    fn test_node_kind_values() {
        assert_eq!(AST_ASM_STMT, 207);
        assert_eq!(ASM_OPERAND_OUTPUT, 208);
        assert_eq!(ASM_OPERAND_INPUT, 209);
        assert_eq!(ASM_CLOBBER, 210);
        assert_eq!(ASM_GOTO_LABEL, 211);
    }

    #[test]
    fn test_asm_volatile_flag_stored() {
        let (_temp, mut parser) = create_parser();
        let source = "int main() { asm volatile(\"nop\"); }";
        let root = parser.parse(source).unwrap();

        let data = find_asm_node_dfs(&parser.arena, root);
        assert!(data.is_some(), "Should find an ASM_STMT node");
        assert!((data.unwrap() & 1) == 1, "volatile flag should be set");
    }
}
