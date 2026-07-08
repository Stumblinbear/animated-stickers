//! Message routing: each `Msg` sub-enum has its own handler module. The
//! actual state mutations and profile edits live on [`App`] (see `app`).

mod canvas;
mod compute;
mod edit;
mod file;
mod layer;
mod profile;
mod ui;

use super::app::App;
use super::msg::Msg;
use iced::Task;

pub fn update(app: &mut App, msg: Msg) -> Task<Msg> {
    match msg {
        Msg::File(m) => file::update(app, m),
        Msg::Layer(m) => layer::update(app, m),
        Msg::Edit(m) => edit::update(app, m),
        Msg::Profile(m) => profile::update(app, m),
        Msg::Ui(m) => ui::update(app, m),
        Msg::Canvas(m) => canvas::update(app, m),
        Msg::Compute(m) => compute::update(app, m),
        Msg::Modifiers(m) => {
            app.modifiers = m;
            Task::none()
        }
        Msg::Tick(now) => {
            app.anim_now = now;
            Task::none()
        }
    }
}
