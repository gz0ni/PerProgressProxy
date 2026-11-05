pub mod app;
pub mod model;
pub mod term;
pub mod view;

use crate::paths::Paths;

pub fn run(paths: &Paths) -> i32 {
    let screen = model::Screen::new(paths.clone());
    let terminal = term::Terminal::new(Some(paths.logs_dir.clone()));
    app::App::new(terminal, screen).run()
}
