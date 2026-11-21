//! One GPUI application and window for the phone routes mounted by Swift.

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::c_void,
    sync::atomic::{AtomicI32, Ordering},
};

use crate::{host::announce, projects::PhoneProjects, Event, Tab};
use gpui_kit::{
    component::Theme, div, prelude::*, px, rgb, rgba, AnyView, AppContext, Application,
    ApplicationHandle, Context, Task, Window, WindowOptions,
};
use sjel_gpui_ui::{
    calendar::PhoneCalendar,
    ely::{
        buttons::{Button, ButtonVariant},
        primitives::{Icon, IconName},
        theme::{ControlSize, IconSize},
    },
    phone_agent::PhoneAgent,
    phone_capture::PhoneCapture,
    phone_edge::{self, PhoneActivity, PhoneScratchpad, Seat},
    phone_finance::PhoneFinance,
    phone_interior::PhoneInterior,
    phone_inventory::PhoneInventory,
    phone_mail::PhoneMail,
    phone_map::PhoneMap,
    phone_navigation::{self, Page},
    phone_people::PhonePeople,
    phone_settings::PhoneSettings,
    phone_soundscape::PhoneSoundscape,
    phone_terminal::PhoneTerminal,
    phone_today::PhoneToday,
    phone_travel::PhoneTravel,
    primitives::back_button,
    theme::{self, Palette, ThemeContext},
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
enum Route {
    Documents = 0,
    Today = 1,
    Capture = 2,
    Projects = 3,
    People = 4,
    Inventory = 5,
    Finance = 6,
    Calendar = 7,
    Settings = 8,
    Scratchpad = 9,
    Activity = 10,
    Soundscape = 11,
    Mail = 12,
    Agent = 13,
    Travel = 14,
    Map = 15,
    Interior = 16,
    Terminal = 17,
}

impl Route {
    fn for_page(page: Page, seat: Seat) -> Self {
        match page {
            Page::Documents => Self::Documents,
            Page::Today => Self::Today,
            Page::Capture => Self::Capture,
            Page::Projects => Self::Projects,
            Page::People => Self::People,
            Page::Inventory => Self::Inventory,
            Page::Finance => Self::Finance,
            Page::Calendar => Self::Calendar,
            Page::Settings => Self::Settings,
            Page::Mail => Self::Mail,
            Page::Travel => Self::Travel,
            Page::Map => Self::Map,
            Page::Interior => Self::Interior,
            Page::Edge => match seat {
                Seat::Scratchpad => Self::Scratchpad,
                Seat::Activity => Self::Activity,
                Seat::Soundscape => Self::Soundscape,
                Seat::Agent => Self::Agent,
                Seat::Terminal => Self::Terminal,
            },
        }
    }
}

const TABS: [(Tab, &str); 12] = [
    (Tab::Today, "Today"),
    (Tab::Mail, "Mail"),
    (Tab::Documents, "Docs"),
    (Tab::Calendar, "Calendar"),
    (Tab::Travel, "Travel"),
    (Tab::Projects, "Projects"),
    (Tab::People, "People & Places"),
    (Tab::Inbox, "Capture"),
    (Tab::Interior, "Interior"),
    (Tab::Finance, "Finance"),
    (Tab::Inventory, "Inventory & Wishlist"),
    (Tab::Settings, "Settings"),
];

static ROUTE: AtomicI32 = AtomicI32::new(-1);

struct PhoneRoot {
    page: Page,
    seat: Seat,
    menu: bool,
    _navigation: Task<()>,
    rendered_route: Option<Route>,
    views: HashMap<Route, AnyView>,
}

impl Render for PhoneRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let route = Route::for_page(self.page, self.seat);
        ROUTE.store(route as i32, Ordering::Relaxed);
        if self.rendered_route != Some(route) {
            window.blur(cx);
            self.rendered_route = Some(route);
        }
        let content = self
            .views
            .entry(route)
            .or_insert_with(|| match route {
                Route::Documents => cx.new(crate::documents::view).into(),
                Route::Today => cx.new(PhoneToday::new).into(),
                Route::Capture => cx.new(|cx| PhoneCapture::new(window, keyboard, cx)).into(),
                Route::Projects => cx.new(PhoneProjects::new).into(),
                Route::People => cx.new(|cx| PhonePeople::new(window, keyboard, cx)).into(),
                Route::Inventory => cx
                    .new(|cx| PhoneInventory::new(window, keyboard, cx))
                    .into(),
                Route::Finance => cx.new(|cx| PhoneFinance::new(window, keyboard, cx)).into(),
                Route::Calendar => cx.new(|cx| PhoneCalendar::new(window, keyboard, cx)).into(),
                Route::Settings => cx.new(PhoneSettings::new).into(),
                Route::Scratchpad => cx.new(|_| PhoneScratchpad).into(),
                Route::Activity => cx.new(PhoneActivity::new).into(),
                Route::Soundscape => cx.new(PhoneSoundscape::new).into(),
                Route::Mail => cx.new(PhoneMail::new).into(),
                Route::Agent => cx.new(|cx| PhoneAgent::new(window, keyboard, cx)).into(),
                Route::Travel => cx.new(|cx| PhoneTravel::new(window, keyboard, cx)).into(),
                Route::Map => cx.new(PhoneMap::new).into(),
                Route::Interior => cx.new(|cx| PhoneInterior::new(window, keyboard, cx)).into(),
                Route::Terminal => cx.new(|cx| PhoneTerminal::new(window, keyboard, cx)).into(),
            })
            .clone();
        let palette = ThemeContext::of(window, cx).palette;
        let panel = self.page == Page::Edge;
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgba(palette.window_tint))
            .text_color(rgb(palette.text))
            .when(panel, |shell| {
                shell.child(
                    div()
                        .flex()
                        .bg(rgba(palette.surface))
                        .border_b_1()
                        .border_color(rgba(palette.border))
                        .children(Seat::ALL.map(|seat| {
                            nav_cell(seat_icon(seat), seat.label(), self.seat == seat, palette)
                                .on_click(move |_, _, _| phone_edge::open(seat))
                        }))
                        .child(
                            nav_cell(IconName::X, "Close", false, palette)
                                .on_click(|_, _, _| phone_edge::close()),
                        ),
                )
            })
            .when(
                !panel
                    && !matches!(
                        self.page,
                        Page::Today
                            | Page::Calendar
                            | Page::Travel
                            | Page::Map
                            | Page::Mail
                            | Page::Documents
                    ),
                |shell| {
                    shell.child(div().px_3().py_1().child(back_button(
                        "Back",
                        |_, _, _| phone_navigation::back(),
                        palette,
                    )))
                },
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(content),
            )
            .when(!panel, |shell| {
                shell
                    .when(self.menu, |shell| {
                        shell.child(
                            div()
                                .flex()
                                .flex_wrap()
                                .p_3()
                                .gap_2()
                                .bg(rgba(palette.raised))
                                .border_t_1()
                                .border_color(rgba(palette.border))
                                .children(TABS[5..].iter().copied().map(|(tab, label)| {
                                    nav_button(tab_icon(tab), label, active(tab, self.page))
                                        .on_click(cx.listener(move |root, _, _, cx| {
                                            root.menu = false;
                                            announce(Event::SelectTab(tab));
                                            cx.notify();
                                        }))
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .bg(rgba(palette.surface))
                            .border_t_1()
                            .border_color(rgba(palette.border))
                            .children(TABS[..5].iter().copied().map(|(tab, label)| {
                                nav_cell(tab_icon(tab), label, active(tab, self.page), palette)
                                    .on_click(cx.listener(move |root, _, _, cx| {
                                        root.menu = false;
                                        announce(Event::SelectTab(tab));
                                        cx.notify();
                                    }))
                            }))
                            .child(
                                nav_cell(IconName::Bot, "Agent", false, palette)
                                    .on_click(|_, _, _| phone_edge::open(Seat::Agent)),
                            )
                            .child(
                                nav_cell(IconName::Ellipsis, "More", self.menu, palette).on_click(
                                    cx.listener(|root, _, _, cx| {
                                        root.menu = !root.menu;
                                        cx.notify();
                                    }),
                                ),
                            ),
                    )
            })
    }
}

fn tab_icon(tab: Tab) -> IconName {
    match tab {
        Tab::Today => IconName::House,
        Tab::Mail => IconName::Mail,
        Tab::Documents => IconName::FileText,
        Tab::Calendar => IconName::Calendar,
        Tab::Travel => IconName::Compass,
        Tab::Projects => IconName::Folder,
        Tab::People => IconName::Users,
        Tab::Inbox => IconName::Inbox,
        Tab::Interior => IconName::House,
        Tab::Finance => IconName::Wallet,
        Tab::Inventory => IconName::Heart,
        Tab::Settings => IconName::Settings,
    }
}

fn seat_icon(seat: Seat) -> IconName {
    match seat {
        Seat::Soundscape => IconName::AudioLines,
        Seat::Activity => IconName::Activity,
        Seat::Agent => IconName::Bot,
        Seat::Scratchpad => IconName::Pencil,
        Seat::Terminal => IconName::Terminal,
    }
}

fn nav_cell(
    icon: IconName,
    label: &'static str,
    active: bool,
    palette: Palette,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let color = rgb(if active {
        palette.accent
    } else {
        palette.muted
    });
    div()
        .id(label)
        .flex_1()
        .min_w_0()
        .min_h(px(50.))
        .py_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_0p5()
        .text_color(color)
        .when(active, |cell| cell.bg(rgba(palette.raised)))
        .child(Icon::new(icon).size(IconSize::Sm).color(color))
        .child(div().text_xs().truncate().child(label))
}

fn nav_button(icon: IconName, label: &'static str, active: bool) -> Button {
    Button::new(label, label)
        .icon(icon)
        .size(ControlSize::Sm)
        .variant(if active {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Secondary
        })
}

fn active(tab: Tab, page: Page) -> bool {
    matches!(
        (tab, page),
        (Tab::Today, Page::Today)
            | (Tab::Mail, Page::Mail)
            | (Tab::Documents, Page::Documents)
            | (Tab::Calendar, Page::Calendar)
            | (Tab::Travel, Page::Travel | Page::Map)
            | (Tab::Projects, Page::Projects)
            | (Tab::People, Page::People)
            | (Tab::Inbox, Page::Capture)
            | (Tab::Interior, Page::Interior)
            | (Tab::Finance, Page::Finance)
            | (Tab::Inventory, Page::Inventory)
            | (Tab::Settings, Page::Settings)
    )
}

impl PhoneRoot {
    fn new(cx: &mut Context<Self>) -> Self {
        let mut pages = phone_navigation::subscribe();
        let mut seats = phone_edge::seat_updates();
        let page = *pages.borrow_and_update();
        let seat = seats.borrow_and_update().seat;
        let navigation = cx.spawn(async move |root, cx| loop {
            let changed = {
                let page = pages.changed();
                let seat = seats.changed();
                futures::pin_mut!(page, seat);
                match futures::future::select(page, seat).await {
                    futures::future::Either::Left((result, _))
                    | futures::future::Either::Right((result, _)) => result,
                }
            };
            if changed.is_err() {
                break;
            }
            let page = *pages.borrow_and_update();
            let seat = seats.borrow_and_update().seat;
            if root
                .update(cx, |root, cx| {
                    if page != root.page || seat != root.seat {
                        root.page = page;
                        root.seat = seat;
                        root.menu = false;
                        cx.notify();
                    }
                })
                .is_err()
            {
                break;
            }
        });
        Self {
            page,
            seat,
            menu: false,
            _navigation: navigation,
            rendered_route: None,
            views: HashMap::new(),
        }
    }
}

thread_local! {
    static APP: RefCell<Option<ApplicationHandle>> = const { RefCell::new(None) };
}

#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_gpui_current_route() -> i32 {
    ROUTE.load(Ordering::Relaxed)
}

pub(crate) fn keyboard(visible: bool) {
    if visible {
        gpui_mobile::show_keyboard();
    } else {
        gpui_mobile::hide_keyboard();
    }
}

/// Updates GPUI and Ely when the Swift host changes its UIKit appearance.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_gpui_apply_theme(system_dark: bool) {
    if objc2::MainThreadMarker::new().is_none() {
        return;
    }
    let appearance = crate::phone_system::appearance();
    APP.with_borrow(|app| {
        if let Some(app) = app {
            app.update(|cx| {
                ThemeContext::sync_with_system(appearance, system_dark, cx);
                let dark = appearance.theme_mode.is_dark(system_dark);
                theme::ely::sync(appearance, dark, cx);
                Theme::update(cx, |theme| {
                    theme.font_size = px(f32::from(appearance.ui_font_size));
                });
                cx.refresh_windows();
            });
        }
    });
}

/// Starts or selects a GPUI route on the UIKit main thread and returns the embedded window.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn sjel_ios_gpui_start() -> *mut c_void {
    use gpui_mobile::ios::ffi;
    if objc2::MainThreadMarker::new().is_none() {
        return std::ptr::null_mut();
    }
    APP.with_borrow_mut(|app| {
        if app.is_some() {
            return;
        }
        crate::host::sjel_ios_phone_prepare();
        ffi::gpui_ios_set_embedded();
        ffi::gpui_ios_initialize();
        let application = Application::with_platform(gpui_mobile::ios::current_platform(false))
            .with_assets(sjel_gpui_ui::ely::Assets::before(
                gpui_kit::assets::Assets::new(""),
            ));
        *app = Some(application.run_embedded(move |cx| {
            gpui_kit::init(cx);
            if let Err(error) = sjel_gpui_ui::ely::init(cx) {
                eprintln!("sjel-ios: GPUI assets: {error}");
                return;
            }
            sjel_gpui_ui::projects::bind_keys(cx);
            sjel_gpui_ui::editor::bind_keys(cx);
            if let Err(error) = cx.open_window(WindowOptions::default(), |window, cx| {
                let root = cx.new(PhoneRoot::new);
                cx.new(|cx| gpui_kit::component::Root::new(root, window, cx))
            }) {
                eprintln!("sjel-ios: GPUI window: {error}");
            }
        }));
        ffi::gpui_ios_did_finish_launching(std::ptr::null_mut());
    });
    ffi::gpui_ios_get_window()
}
