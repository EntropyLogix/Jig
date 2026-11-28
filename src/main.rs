use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("blt: {e}");
            std::process::exit(1);
        }
    };
    if args.len() == 1 && args[0] == "mcp" {
        if let Err(error) = billet::mcp::serve(&cwd) {
            eprintln!("blt: {error}");
            std::process::exit(1);
        }
        return;
    }
    match billet::commands::run(&args, &cwd) {
        Ok(out) => {
            for warning in &out.warnings {
                eprintln!("blt: warning: {warning}");
            }
            let written = std::io::stdout()
                .lock()
                .write_all(out.stdout.as_bytes())
                .and_then(|()| std::io::stdout().flush());
            match written {
                Ok(()) => std::process::exit(out.code),
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => std::process::exit(0),
                Err(e) => {
                    eprintln!("blt: write stdout: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("blt: {}", e.msg);
            std::process::exit(e.code);
        }
    }
}
