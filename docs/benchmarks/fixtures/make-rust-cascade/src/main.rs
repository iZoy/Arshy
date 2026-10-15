fn parse_port(s: &str) -> u16 {
    s // E0308
}

fn main() {
    let p = parse_port("8080");
    // E0425
    bind(p);
}
