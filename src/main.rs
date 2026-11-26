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
    match jig::commands::run(&args, &cwd) {
        Ok(out) => {
            for warning in &out.warnings {
                eprintln!("jig: warning: {warning}");
            }
            let written = std::io::stdout()
                .lock()
                .write_all(out.stdout.as_bytes())
                .and_then(|()| std::io::stdout().flush());
            match written {
                Ok(()) => std::process::exit(out.code),
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => std::process::exit(0),
                Err(e) => {
                    eprintln!("jig: write stdout: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("jig: {}", e.msg);
            std::process::exit(e.code);
        }
    }
}
