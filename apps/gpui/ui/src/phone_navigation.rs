//! Phone destinations and back history, independent of the rendering host.

use std::sync::{Mutex, OnceLock};

use tokio::sync::watch;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Today,
    Calendar,
    Map,
    Travel,
    Projects,
    Interior,
    Edge,
    Capture,
    People,
    Settings,
    Documents,
    Mail,
    Finance,
    Inventory,
}

struct Router {
    page: watch::Sender<Page>,
    history: Mutex<Vec<Page>>,
}

fn router() -> &'static Router {
    static ROUTER: OnceLock<Router> = OnceLock::new();
    ROUTER.get_or_init(|| Router {
        page: watch::channel(Page::Today).0,
        history: Mutex::new(Vec::new()),
    })
}

pub fn page() -> Page {
    *router().page.borrow()
}

pub fn subscribe() -> watch::Receiver<Page> {
    router().page.subscribe()
}

pub fn open(target: Page) {
    let previous = page();
    if previous == target {
        return;
    }
    if previous != Page::Edge {
        let mut history = router().history.lock().unwrap_or_else(|e| e.into_inner());
        if history.last().copied() != Some(previous) {
            history.push(previous);
            if history.len() > 16 {
                history.remove(0);
            }
        }
    }
    change(previous, target);
}

pub fn back() {
    let previous = page();
    if previous == Page::Edge {
        crate::phone_edge::close();
        return;
    }
    let target = router()
        .history
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .pop()
        .unwrap_or_default();
    change(previous, target);
    if target == Page::Map {
        crate::phone_map::show(None);
    }
}

fn change(previous: Page, target: Page) {
    if previous == Page::Edge && target != Page::Edge {
        crate::phone_edge::leaving();
    }
    if previous == Page::Map && target != Page::Map {
        crate::phone_map::hide();
    }
    router().page.send_replace(target);
}
