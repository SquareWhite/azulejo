//! Azulejo GTK4 shell. Builds on Linux only; excluded from default workspace members.

use adw::prelude::*;
use libadwaita as adw;

const APP_ID: &str = "dev.azulejo.Azulejo";

fn main() -> gtk4::glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(|app| {
        let label = gtk4::Label::new(Some("Azulejo"));
        adw::ApplicationWindow::builder()
            .application(app)
            .title("Azulejo")
            .default_width(960)
            .default_height(640)
            .content(&label)
            .build()
            .present();
    });
    app.run()
}
