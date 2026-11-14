//! `azj`: command-line client for the Azulejo socket API.

fn main() {
    let args = std::env::args().skip(1).collect();
    let env = azulejo_cli::Env::from_process();
    let code = azulejo_cli::run(
        args,
        &env,
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    std::process::exit(code);
}
