mod util;

fn greet(name: &str) -> String {
    // E0308: returns &str instead of String
    name
}

fn main() {
    // E0425: missing function
    let _ = missing_helper(1);
    println!("{}", greet("hi"));
    util::answer();
}
