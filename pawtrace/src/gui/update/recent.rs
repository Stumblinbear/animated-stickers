//! Welcome-screen recents: filter, category, and per-item open and pin.

use crate::gui::app::App;
use crate::gui::msg::{FileMsg, Msg, RecentMsg};
use iced::Task;

pub(super) fn update(app: &mut App, msg: RecentMsg) -> Task<Msg> {
    match msg {
        RecentMsg::Tab(t) => {
            app.welcome.tab = t;
            Task::none()
        }
        RecentMsg::Search(s) => {
            app.welcome.search = s;
            Task::none()
        }
        RecentMsg::Pin(i) => {
            app.toggle_recent_pin(i);
            Task::none()
        }
        RecentMsg::Open(i) => match app.recent_path(i) {
            // A recent folder re-scans and re-records the folder; a recent file
            // opens directly.
            Some(path) if path.is_dir() => Task::done(Msg::File(FileMsg::OpenedFolder(path))),
            Some(path) => Task::done(Msg::File(FileMsg::Opened(vec![path]))),
            None => Task::none(),
        },
    }
}
