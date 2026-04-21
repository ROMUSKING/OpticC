use optic_c::frontend::parser::Parser;
use optic_c::arena::Arena;
use tempfile::NamedTempFile;

fn main() {
    let source = "struct Methods { int (*xAlloc)(int); void (*xFree)(void*); };";
    let temp_file = NamedTempFile::new().unwrap();
    let arena = Arena::new(temp_file.path(), 65536).unwrap();
    let mut parser = Parser::new(arena);
    let root = parser.parse(source).expect("parse failed");
    optic_c::arena::dump_ast(&parser.arena, root, 0);
}
