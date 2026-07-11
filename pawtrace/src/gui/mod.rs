//! iced UI (feature "gui"): a per-layer pipeline editor and profile manager.
//! Documents open as tabs, each carrying its own selection, view, and cached
//! compute; edits re-run the pipeline in the background and stream in.
//!
//! Split: `doc` loads files, `compute` runs the pipeline off the UI thread,
//! `update` routes messages, `view` builds the widget tree, and `app` owns
//! the state and the profile-edit routing.

mod app;
mod compute;
mod doc;
mod edit_target;
mod fields;
mod ids;
mod msg;
mod overlays;
mod phases;
mod profile_ops;
mod recents;
mod tools;
mod undo;
mod update;
mod view;

use app::App;
use iced::Task;
use msg::{FileMsg, Msg, UiMsg};

mod snapshot;
pub use snapshot::{write_snapshot, Scene};

pub fn run(initial: Vec<std::path::PathBuf>) -> iced::Result {
    iced::application(
        move || {
            let mut app = App::default();
            // Load the persisted recents only in the real app. Snapshots build
            // from a pure Default, so this stays out of the headless harness.
            app.welcome.recents = recents::load();
            (app, Task::done(Msg::File(FileMsg::Opened(initial.clone()))))
        },
        update::update,
        view::view,
    )
    .title("Pawtrace")
    .antialiasing(true)
    .theme(theme)
    .subscription(subscription)
    .font(include_bytes!("../../assets/lucide.ttf").as_slice())
    .run()
}

// A named function, not a closure: `.theme(|_| …)` trips a higher-ranked
// lifetime inference bug in iced 0.14.
fn theme(_: &App) -> iced::Theme {
    view::theme()
}

fn subscription(app: &App) -> iced::Subscription<Msg> {
    use iced::keyboard;
    let modifiers = iced::event::listen_with(|event, _status, _window| match event {
        iced::Event::Keyboard(keyboard::Event::ModifiersChanged(m)) => Some(Msg::Modifiers(m)),
        _ => None,
    });
    let shortcuts = iced::event::listen_with(|event, _status, _window| match event {
        iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
            shortcut(key, modifiers)
        }
        _ => None,
    });
    // Only subscribed while something is processing, so at rest the window
    // requests no frames and stays idle.
    let frames = if app.is_animating() {
        iced::window::frames().map(Msg::Tick)
    } else {
        iced::Subscription::none()
    };
    iced::Subscription::batch([modifiers, shortcuts, frames])
}

/// Maps a Ctrl/Cmd key chord to its command.
fn shortcut(key: iced::keyboard::Key, m: iced::keyboard::Modifiers) -> Option<Msg> {
    use iced::keyboard::Key;
    if !(m.control() || m.command()) {
        return None;
    }
    let Key::Character(c) = key else {
        return None;
    };
    let msg = match c.as_str() {
        "o" if m.shift() => Msg::File(FileMsg::OpenFolder),
        "o" => Msg::File(FileMsg::OpenFiles),
        "s" => Msg::File(FileMsg::SaveProfiles),
        "e" => Msg::File(FileMsg::ExportAll),
        "z" if m.shift() => Msg::Edit(msg::EditMsg::Redo),
        "z" => Msg::Edit(msg::EditMsg::Undo),
        "y" => Msg::Edit(msg::EditMsg::Redo),
        // The subscription can't see the selected tab; `None` lets the handler
        // resolve it.
        "w" => Msg::File(FileMsg::CloseDoc(None)),
        // "=" is the unshifted key that prints "+"; accept both so Ctrl++ works
        // whether or not shift is held.
        "=" | "+" => Msg::Ui(UiMsg::ZoomIn),
        "-" => Msg::Ui(UiMsg::ZoomOut),
        "0" => Msg::Ui(UiMsg::ZoomFit),
        _ => return None,
    };
    Some(msg)
}
