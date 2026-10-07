//! The modules of the application: each is a feature that registers its commands.

mod app;
mod edit;
pub mod file;
pub mod find;
mod view;
mod window;

use crate::commands::Module;

/// Every module; within a group of a menu, items go in this order.
pub fn all() -> [&'static dyn Module; 6] {
    [&app::AppModule, &file::FileModule, &edit::EditModule, &find::FindModule, &view::ViewModule, &window::WindowModule]
}
