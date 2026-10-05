//! The modules of the application: each is a feature that registers its commands.

mod app;
mod edit;
mod view;
mod window;

use crate::commands::Module;

/// Every module; within a group of a menu, items go in this order.
pub fn all() -> [&'static dyn Module; 4] {
    [&app::AppModule, &edit::EditModule, &view::ViewModule, &window::WindowModule]
}
