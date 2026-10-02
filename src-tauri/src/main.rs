// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 3 && args[1] == "--transcribe" {
        if let Err(e) = murmur_lib::cli_transcribe(&args[2]) {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
        return;
    }
    if args.len() == 2 && args[1] == "--focus-test" {
        murmur_lib::cli_focus_test();
        return;
    }
    murmur_lib::run();
}
