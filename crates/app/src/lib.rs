//! The rsdisk window. Logic that can be tested without a window lives in plain modules here;
//! the views only draw them and forward input.

pub mod browser;
pub mod colour;
pub mod keys;
pub mod layout;
mod list_view;
mod scanning;
pub mod settings;
pub mod theme;
pub mod view;
