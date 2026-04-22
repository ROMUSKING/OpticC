use optic_c::arena::{Arena, NodeOffset};
use optic_c::frontend::lexer::Lexer;
use optic_c::frontend::parser::Parser;
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let src = std::fs::read_to_string(&args[1]).unwrap();
    let arena = Arena::new(std::path::PathBuf::from("/tmp/optic_arena.bin"), 1024 * 1024).unwrap();
    let mut lexer = Lexer::new(src.as_bytes());
    let mut tokens = Vec::new();
    loop {
        let t = lexer.next_token();
        tokens.push(t.clone());
        if t.kind == optic_c::frontend::lexer::TokenKind::Eof { break; }
    }
    let mut parser = Parser::new(arena);
    let root = parser.parse_tokens(&tokens);
    
    fn dfs(arena: &Arena, offset: NodeOffset, depth: usize) {
        if offset == NodeOffset::NULL { return; }
        if let Some(n) = arena.get(offset) {
            let data_str = if n.data > 0 {
                arena.get_string(NodeOffset(n.data)).unwrap_or("")
            } else {
                ""
            };
            println!("{:indent$}kind={} data={} ({})", "", n.kind, n.data, data_str, indent=depth*2);
            dfs(arena, n.first_child, depth + 1);
            dfs(arena, n.next_sibling, depth);
        }
    }
    dfs(&parser.arena, root.unwrap(), 0);
}
