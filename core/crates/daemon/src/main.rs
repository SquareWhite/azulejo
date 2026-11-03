//! `azulejod`: the core without a GUI, for development, scripting and the
//! end-to-end tests. Serves the API until SIGINT or SIGTERM, then saves state
//! and removes its socket.
//!
//! ```text
//! azulejod [--state PATH | --no-state] [--socket PATH] [--config PATH | --no-config]
//!          [--herdr-session NAME | --herdr-session default | --no-herdr]
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use azulejo_daemon::{App, HerdrChoice, Options};
use azulejo_herdr::Session as HerdrSession;

const USAGE: &str = "usage: azulejod [--state PATH | --no-state] [--socket PATH] \
[--config PATH | --no-config] [--herdr-session NAME|default | --no-herdr]";

fn parse(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut opts = Options {
        api_required: true,
        ..Options::default()
    };
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
            _ => (arg.clone(), None),
        };
        let mut value = || {
            inline
                .clone()
                .or_else(|| args.next())
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--state" => opts.state_path = Some(PathBuf::from(value()?)),
            "--no-state" => opts.state_path = None,
            "--socket" => opts.socket_path = Some(PathBuf::from(value()?)),
            "--config" => opts.config_path = Some(PathBuf::from(value()?)),
            "--no-config" => opts.config_path = None,
            "--herdr-session" => {
                let name = value()?;
                opts.herdr = HerdrChoice::Session(if name == "default" {
                    HerdrSession::default_session()
                } else {
                    HerdrSession::named(name)
                });
            }
            "--no-herdr" => opts.herdr = HerdrChoice::Off,
            "-h" | "--help" => return Err(String::new()),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(opts)
}

fn main() -> ExitCode {
    let opts = match parse(std::env::args().skip(1)) {
        Ok(opts) => opts,
        Err(e) if e.is_empty() => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("azulejod: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let app = match App::start(opts) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("azulejod: {e}");
            return ExitCode::FAILURE;
        }
    };
    let socket = app
        .socket()
        .map_or("(no API)".to_owned(), |p| p.display().to_string());
    match app.herdr() {
        Some(h) => eprintln!(
            "azulejod: serving {socket}; herdr session {} at {}",
            h.session.name().unwrap_or("default"),
            h.socket.display()
        ),
        None => eprintln!("azulejod: serving {socket}; no herdr"),
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    runtime.block_on(async {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = signal(SignalKind::terminate()).expect("SIGTERM handler");
        let mut int = signal(SignalKind::interrupt()).expect("SIGINT handler");
        tokio::select! {
            _ = term.recv() => {}
            _ = int.recv() => {}
        }
    });
    eprintln!("azulejod: shutting down");
    app.shutdown();
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|s| (*s).to_owned())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn flags_parse() {
        let o = parse(args(&[
            "--state=/tmp/s.json",
            "--socket",
            "/tmp/a.sock",
            "--no-config",
            "--herdr-session",
            "azj-e2e",
        ]))
        .unwrap();
        assert_eq!(o.state_path, Some("/tmp/s.json".into()));
        assert_eq!(o.socket_path, Some("/tmp/a.sock".into()));
        assert_eq!(o.config_path, None);
        assert_eq!(
            o.herdr,
            HerdrChoice::Session(HerdrSession::named("azj-e2e"))
        );
        assert!(o.api_required);

        let o = parse(args(&["--no-herdr", "--herdr-session=default"])).unwrap();
        assert_eq!(
            o.herdr,
            HerdrChoice::Session(HerdrSession::default_session())
        );
        assert!(parse(args(&["--bogus"])).is_err());
        assert!(parse(args(&["--state"])).is_err());
    }
}
