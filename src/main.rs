use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("jig: {e}");
            std::process::exit(1);
        }
    };
    match entropyjig::commands::run(&args, &cwd) {
        Ok(out) => {
            if let Err(e) = std::io::stdout().lock().write_all(out.as_bytes()) {
                eprintln!("jig: write stdout: {e}");
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("jig: {}", e.msg);
            std::process::exit(e.code);
        }
    }
}
