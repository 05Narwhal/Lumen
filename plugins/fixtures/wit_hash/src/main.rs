//! Prints the blake3 (hex) of the WIT file given as the first argument, CRLF folded to LF, so a
//! checkout with Windows line endings hashes the same (photos-plugins' `wit_hash_matches_fixtures`).

fn main() {
    let path = std::env::args().nth(1).expect("usage: wit_hash <file.wit>");
    let text = std::fs::read_to_string(path).expect("read the WIT");
    let text = text.replace("\r\n", "\n");
    println!("{}", blake3::hash(text.as_bytes()).to_hex());
}
