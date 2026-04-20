use optic_c::frontend::parser::Parser;
use optic_c::arena::Arena;
use tempfile::NamedTempFile;

fn main() {
    let source = "int test_sizeof(void) { return sizeof(int); }";
    let temp_file = NamedTempFile::new().unwrap();
    let arena = Arena::new(temp_file.path(), 65536).unwrap();
    let mut parser = Parser::new(arena);
    let root = parser.parse(source).expect("parse failed");
    println!("AST Root: {:?}", root);
    // dump ast
    optic_c::arena::dump_ast(&parser.arena, root, 0);
}
