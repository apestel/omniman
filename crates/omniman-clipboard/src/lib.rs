pub mod store;
pub mod watcher;

pub use store::ClipboardStore;
pub use watcher::read_text;
pub use watcher::spawn as spawn_watcher;
