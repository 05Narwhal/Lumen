#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Before the window and the single-instance check: this process only pipes stdio to the
    // Lumen already running (Round 16).
    if std::env::args().any(|a| a == "--mcp") {
        std::process::exit(lumen_lib::mcp_stdio());
    }
    // The release script's check (Round 17): migrates a temp copy, never the library.
    let args: Vec<String> = std::env::args().collect();
    if let Some(at) = args.iter().position(|a| a == "--check-library") {
        let Some(root) = args.get(at + 1) else {
            eprintln!("usage: Lumen --check-library <library folder>");
            std::process::exit(2);
        };
        std::process::exit(lumen_lib::check_library(std::path::Path::new(root)));
    }
    // Lumen Setup's close request (Round 18 A2) with no Lumen open: nothing to do, no window.
    if args.iter().skip(1).any(|a| a == "--quit") && lumen_lib::quit_finds_nothing() {
        std::process::exit(0);
    }
    lumen_lib::run()
}
