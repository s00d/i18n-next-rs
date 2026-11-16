//! Native labels bound to texts (DESIGN 6.3). What to relabel and when is decided here,
//! apart from the Tauri calls (7.5): [`Bindings`] renders every bound text after a change
//! and returns only the labels that differ from what the target shows.

use i18n_next::Text;
use std::sync::Arc;

/// Something with a label the plugin can set: menu items, submenus, the tray tooltip,
/// window titles. Implement it for your own targets (a custom widget, a test double).
pub trait Relabel: Send + Sync {
    /// Shows `label`.
    ///
    /// # Errors
    /// What Tauri returns (a destroyed item or window).
    fn relabel(&self, label: &str) -> tauri::Result<()>;
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
macro_rules! relabel_text {
    ($($ty:ident),*) => {$(
        impl<R: tauri::Runtime> Relabel for tauri::menu::$ty<R> {
            fn relabel(&self, label: &str) -> tauri::Result<()> {
                self.set_text(label)
            }
        }
    )*};
}
#[cfg(not(any(target_os = "android", target_os = "ios")))]
relabel_text!(MenuItem, CheckMenuItem, IconMenuItem, Submenu);

/// The tooltip (feature `tray-icon`, as tauri's).
#[cfg(all(
    feature = "tray-icon",
    not(any(target_os = "android", target_os = "ios"))
))]
impl<R: tauri::Runtime> Relabel for tauri::tray::TrayIcon<R> {
    fn relabel(&self, label: &str) -> tauri::Result<()> {
        self.set_tooltip(Some(label))
    }
}

impl<R: tauri::Runtime> Relabel for tauri::WebviewWindow<R> {
    fn relabel(&self, label: &str) -> tauri::Result<()> {
        self.set_title(label)
    }
}

impl<R: tauri::Runtime> Relabel for tauri::Window<R> {
    fn relabel(&self, label: &str) -> tauri::Result<()> {
        self.set_title(label)
    }
}

/// A binding made by [`crate::Handle::bind`], for [`crate::Handle::unbind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindId(u64);

struct Bound {
    id: u64,
    target: Arc<dyn Relabel>,
    text: Text,
    /// What the target shows now.
    shown: String,
}

/// The bound targets, in bind order.
#[derive(Default)]
pub(crate) struct Bindings {
    next: u64,
    list: Vec<Bound>,
}

impl Bindings {
    /// Binds `text`; returns its id and the label to show now.
    pub(crate) fn add(&mut self, target: Arc<dyn Relabel>, text: Text) -> (BindId, String) {
        self.next += 1;
        let shown = text.to_string();
        self.list.push(Bound {
            id: self.next,
            target,
            text,
            shown: shown.clone(),
        });
        (BindId(self.next), shown)
    }

    pub(crate) fn remove(&mut self, id: BindId) -> bool {
        self.list.extract_if(.., |b| b.id == id.0).count() > 0
    }

    /// Renders every bound text again: the targets whose label changed, with it.
    pub(crate) fn refresh(&mut self) -> Vec<(Arc<dyn Relabel>, String)> {
        let mut todo = Vec::new();
        for b in &mut self.list {
            let now = b.text.to_string();
            if now != b.shown {
                b.shown.clone_from(&now);
                todo.push((b.target.clone(), now));
            }
        }
        todo
    }
}
