//! Azulejo GTK4 shell. Builds on Linux only; excluded from default workspace members.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use azulejo_engine::Engine;
use azulejo_protocol::{Axis, Command, Direction, Event, LayoutNode};
use gtk4::{gdk, glib};
use libadwaita as adw;

const APP_ID: &str = "dev.azulejo.Azulejo";

const CSS: &str = "
.tile { color: white; font-family: monospace; font-size: 20px; font-weight: 600; border-radius: 6px; }
.tile.focused { box-shadow: inset 0 0 0 3px white; }
.tile-0 { background: #4f8f9a; } .tile-1 { background: #5b63c0; }
.tile-2 { background: #c07a3c; } .tile-3 { background: #b93f55; }
.tile-4 { background: #57a457; } .tile-5 { background: #9a3fb5; }
.tile-6 { background: #8a6a4a; } .tile-7 { background: #3f7fd0; }
";

/// Owns the core and the window. The window's content is rebuilt from
/// `layout.changed` events only; the shell keeps no layout state of its own.
struct Shell {
    engine: RefCell<Engine>,
    window: adw::ApplicationWindow,
}

impl Shell {
    fn send(&self, command: Command) {
        let events = self.engine.borrow_mut().handle(command);
        for event in events {
            self.apply(event);
        }
    }

    fn apply(&self, event: Event) {
        if let Event::LayoutChanged { tree, focused } = event {
            let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
            root.set_margin_top(4);
            root.set_margin_bottom(4);
            root.set_margin_start(4);
            root.set_margin_end(4);
            root.append(&build(&tree, &focused));
            self.window.set_content(Some(&root));
        }
    }
}

/// Spike-grade (step 0.6): a split is a homogeneous Box, so every ratio draws as
/// 0.5. The real `TileLayout` widget in step 3.1 honours ratios.
fn build(node: &LayoutNode, focused: &str) -> gtk4::Widget {
    match node {
        LayoutNode::Leaf { id } => {
            let label = gtk4::Label::new(Some(id));
            label.set_hexpand(true);
            label.set_vexpand(true);
            label.add_css_class("tile");
            let n: usize = id.trim_start_matches('t').parse().unwrap_or(0);
            label.add_css_class(&format!("tile-{}", n % 8));
            if id == focused {
                label.add_css_class("focused");
            }
            label.upcast()
        }
        LayoutNode::Split { axis, a, b, .. } => {
            let orientation = match axis {
                Axis::Horizontal => gtk4::Orientation::Horizontal,
                Axis::Vertical => gtk4::Orientation::Vertical,
            };
            let split = gtk4::Box::new(orientation, 4);
            split.set_homogeneous(true);
            split.append(&build(a, focused));
            split.append(&build(b, focused));
            split.upcast()
        }
    }
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        let provider = gtk4::CssProvider::new();
        provider.load_from_string(CSS);
        gtk4::style_context_add_provider_for_display(
            &gdk::Display::default().expect("no display"),
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
    app.connect_activate(|app| {
        let window = adw::ApplicationWindow::builder()
            .application(app)
            .title("Azulejo")
            .default_width(960)
            .default_height(640)
            .build();
        let shell = Rc::new(Shell {
            engine: RefCell::new(Engine::new()),
            window: window.clone(),
        });

        // Ctrl+D splits right, Ctrl+Shift+D splits down. A placeholder for the real key router.
        let keys = gtk4::EventControllerKey::new();
        keys.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let handler = Rc::clone(&shell);
        keys.connect_key_pressed(move |_, key, _, state| {
            let mods = state
                & (gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::SHIFT_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK);
            let direction = match (key.to_lower(), mods) {
                (gdk::Key::d, m) if m == gdk::ModifierType::CONTROL_MASK => Direction::Right,
                (gdk::Key::d, m)
                    if m == gdk::ModifierType::CONTROL_MASK | gdk::ModifierType::SHIFT_MASK =>
                {
                    Direction::Down
                }
                _ => return glib::Propagation::Proceed,
            };
            handler.send(Command::LayoutSplit { direction });
            glib::Propagation::Stop
        });
        window.add_controller(keys);

        // Spike (step 0.6): seed the same splits as the macOS shell.
        shell.send(Command::LayoutGet);
        shell.send(Command::LayoutSplit {
            direction: Direction::Right,
        });
        shell.send(Command::LayoutSplit {
            direction: Direction::Down,
        });
        shell.send(Command::LayoutSplit {
            direction: Direction::Right,
        });
        window.present();
    });
    app.run()
}
