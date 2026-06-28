fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(|s| s.as_str()) {
        Some("help") | None => print_help(),
        Some(cmd) => {
            eprintln!("unknown command: {cmd}");
            eprintln!();
            print_help();
            std::process::exit(1);
        }
    }
}

fn print_help() {
    println!("poker-hands-storage-tools");
    println!();
    println!("USAGE:");
    println!("    poker-hands-storage-tools <COMMAND>");
    println!();
    println!("COMMANDS:");
    println!("    help    Print this help message");
}
