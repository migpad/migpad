//! The modules of the application: each is a feature that registers its commands.

mod app;
mod edit;
pub mod file;
pub mod find;
pub mod format;
pub mod go_to;
pub mod transform;
mod view;
mod window;

use crate::commands::Module;

/// Every module; within a group of a menu, items go in this order.
pub fn all() -> [&'static dyn Module; 9] {
    [
        &app::AppModule,
        &file::FileModule,
        &edit::EditModule,
        &transform::TransformModule,
        &find::FindModule,
        &go_to::GoToModule,
        &format::FormatModule,
        &view::ViewModule,
        &window::WindowModule,
    ]
}
