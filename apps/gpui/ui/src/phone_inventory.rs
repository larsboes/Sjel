//! Phone inventory data read from the paired Mac.

use std::sync::OnceLock;

use sjel_gpui_core::{
    device::Source,
    inventory::{format_cents, InventoryItem, InventoryRow, InventoryWishlist, WishGroup},
    shell,
};
use tokio::sync::watch;

use crate::phone_feed::{SourceFeed, SourceState};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InventoryTab {
    #[default]
    Wishlist,
    Owned,
    Compare,
}

impl InventoryTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Wishlist => "Wishlist",
            Self::Owned => "Owned",
            Self::Compare => "Compare",
        }
    }
}

#[derive(Clone)]
pub struct Snapshot {
    pub rows: Vec<InventoryRow>,
    pub wishlist: InventoryWishlist,
    pub groups: Vec<WishGroup>,
    pub status: String,
    pub loading: bool,
    pub error: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            wishlist: InventoryWishlist {
                items: Vec::new(),
                total_low_cents: 0,
                total_high_cents: 0,
                unpriced_count: 0,
                monthly_balance: None,
                months_to_pay: None,
            },
            groups: Vec::new(),
            status: "Connect to the paired Mac to read inventory and wishlist.".into(),
            loading: false,
            error: None,
        }
    }
}

fn feed() -> &'static SourceFeed<Snapshot> {
    static FEED: OnceLock<SourceFeed<Snapshot>> = OnceLock::new();
    FEED.get_or_init(SourceFeed::new)
}

fn state() -> std::sync::MutexGuard<'static, SourceState<Snapshot>> {
    feed().state()
}

fn publish(s: &SourceState<Snapshot>) {
    feed().publish(s);
}

pub(super) fn snapshots() -> &'static watch::Sender<Snapshot> {
    feed().snapshots()
}

/// Connects the phone Inventory screen to the Mac's shell or paired device client.
pub fn connect(source: Source) {
    {
        let mut s = state();
        s.source = Some(source);
    }
    load();
}

/// Sets the inventory and wishlist snapshot directly (used by offline caching and tests).
pub fn set_snapshot(rows: Vec<InventoryRow>, wishlist: InventoryWishlist, groups: Vec<WishGroup>) {
    let mut s = state();
    let owned_count = rows
        .iter()
        .filter(|r| r.state.as_deref() == Some("owned"))
        .count();
    let wish_count = wishlist.items.len();
    let group_count = groups.len();
    s.snap.rows = rows;
    s.snap.wishlist = wishlist;
    s.snap.groups = groups;
    s.snap.loading = false;
    s.snap.error = None;
    s.snap.status = format!("{wish_count} wishes · {owned_count} owned · {group_count} groups");
    publish(&s);
}

/// Refreshes inventory rows, wishlist, and wish groups from the connected source.
pub fn load() {
    let source = {
        let mut s = state();
        let Some(source) = s.source.clone() else {
            return;
        };
        s.snap.loading = true;
        s.snap.error = None;
        s.snap.status = "Reading inventory & wishlist from the Mac…".into();
        publish(&s);
        source
    };
    shell::spawn(async move {
        let (rows_res, wish_res, groups_res) = futures::join!(
            source.inventory_rows(),
            source.inventory_wishlist(),
            source.inventory_wish_groups(),
        );
        let mut s = state();
        s.snap.loading = false;
        if let Ok(rows) = rows_res {
            s.snap.rows = rows;
        }
        match wish_res {
            Ok(wishlist) => {
                s.snap.wishlist = wishlist;
                if let Ok(groups) = groups_res {
                    s.snap.groups = groups;
                }
                let owned_count = s
                    .snap
                    .rows
                    .iter()
                    .filter(|r| r.state.as_deref() == Some("owned"))
                    .count();
                s.snap.status = format!(
                    "{} wishes · {owned_count} owned · {} groups",
                    s.snap.wishlist.items.len(),
                    s.snap.groups.len()
                );
            }
            Err(err) => {
                s.snap.error = Some(err);
            }
        }
        publish(&s);
    });
}

pub(super) fn format_item_price(item: &InventoryItem) -> String {
    if let Some(price) = item.price_cents {
        format_cents(Some(price))
    } else if item.cost_min_cents.is_some() || item.cost_max_cents.is_some() {
        format!(
            "{}–{}",
            format_cents(item.cost_min_cents),
            format_cents(item.cost_max_cents)
        )
    } else {
        "Unpriced".into()
    }
}

mod view;
pub use view::PhoneInventory;
