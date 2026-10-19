//! The rsdisk window. Logic that can be tested without a window lives in plain modules here;
//! the views only draw them and forward input.

pub mod browser;
mod cleanup;
pub mod colour;
mod find;
pub mod keys;
pub mod layout;
mod list_view;
pub mod marks;
pub mod permissions;
mod quick_look;
mod rescan;
mod scanning;
pub mod search;
pub mod settings;
pub mod settings_window;
pub mod theme;
pub mod view;
mod visual;
