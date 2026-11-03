//! Spike 0.7: a GTK4 window with one VTE terminal running an arbitrary
//! command (in practice `herdr terminal attach <id>`).
//!
//! Usage: azj-vte-spike -- <program> [args...]

use gtk::prelude::*;
use gtk::{gio, glib};
use vte::prelude::*;

fn main() -> glib::ExitCode {
    let argv: Vec<String> = std::env::args().skip_while(|a| a != "--").skip(1).collect();
    if argv.is_empty() {
        eprintln!("usage: azj-vte-spike -- <program> [args...]");
        return glib::ExitCode::FAILURE;
    }

    let app = gtk::Application::builder()
        .application_id("dev.azulejo.VteSpike")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();

    app.connect_activate(move |app| {
        let term = vte::Terminal::new();
        term.set_font(Some(&gtk::pango::FontDescription::from_string(
            "JetBrains Mono 12",
        )));
        term.set_scrollback_lines(10_000);

        let argv_ref: Vec<&str> = argv.iter().map(String::as_str).collect();
        let envv: Vec<String> = std::env::vars()
            .map(|(k, v)| format!("{k}={v}"))
            .chain(["TERM=xterm-256color".to_string()])
            .collect();
        let envv_ref: Vec<&str> = envv.iter().map(String::as_str).collect();

        term.spawn_async(
            vte::PtyFlags::DEFAULT,
            None,
            &argv_ref,
            &envv_ref,
            glib::SpawnFlags::SEARCH_PATH,
            || {},
            -1,
            gio::Cancellable::NONE,
            |res| match res {
                Ok(pid) => eprintln!("spawned pid {pid:?}"),
                Err(e) => eprintln!("spawn failed: {e}"),
            },
        );

        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .title("azulejo vte spike")
            .default_width(1000)
            .default_height(620)
            .child(&term)
            .build();
        term.connect_child_exited(|t, status| {
            eprintln!("child exited: {status}");
            if let Some(w) = t.root().and_downcast::<gtk::Window>() {
                w.close();
            }
        });
        window.present();
        term.grab_focus();
    });

    app.run_with_args(&[] as &[&str])
}
