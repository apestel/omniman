pub mod store;
pub mod watcher;

pub use store::ClipboardStore;
pub use watcher::spawn as spawn_watcher;
