//! The reusable PDF page view: zoom levels, the scrollable page element and its zoom controls.
//!
//! A screen keeps its own page state and renderer requests; this module decides how a drawn page
//! is laid out and which bitmap width a zoom level needs. Every PDF surface in the app uses it,
//! so fit, scroll and zoom behave the same everywhere.

use std::rc::Rc;
use std::sync::Arc;

use crate::ely::buttons::Button;
use crate::ely::theme::ControlSize;
use gpui::{
    div, img, prelude::*, px, AnyElement, App, ElementId, ObjectFit, RenderImage, SharedString,
    StyledImage, Window,
};

/// Logical width of a page at 100 %. Close to an A4 page on a Mac display.
const BASE_WIDTH: f32 = 800.;
/// Bitmaps are drawn at twice the logical width for Retina displays.
const PIXELS_PER_POINT: f32 = 2.;
/// The percent steps of `+` and `−`.
const STEPS: [u16; 8] = [50, 75, 100, 125, 150, 200, 300, 400];
/// The bitmap width for both fit modes; wide enough to stay sharp in a full-width pane.
const FIT_PIXELS: u32 = 1600;

/// How the page is laid out. The bitmap is drawn at the width the level needs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Zoom {
    /// The whole page inside the pane.
    Page,
    /// The page as wide as the pane; scroll down through it.
    #[default]
    Width,
    /// A fixed size in percent of [`BASE_WIDTH`]; scroll both ways.
    Percent(u16),
}

impl Zoom {
    /// The bitmap width in pixels the renderer draws for this level.
    pub fn width(self) -> u32 {
        match self {
            Self::Page | Self::Width => FIT_PIXELS,
            Self::Percent(percent) => {
                (BASE_WIDTH * PIXELS_PER_POINT * f32::from(percent) / 100.) as u32
            }
        }
    }

    pub fn label(self) -> SharedString {
        match self {
            Self::Page => "Fit page".into(),
            Self::Width => "Fit width".into(),
            Self::Percent(percent) => format!("{percent}%").into(),
        }
    }

    /// The next larger step; a fit mode continues from 100 %.
    pub fn zoomed_in(self) -> Self {
        let current = self.percent().unwrap_or(STEPS[1]);
        STEPS
            .iter()
            .find(|step| **step > current)
            .map_or(self, |step| Self::Percent(*step))
    }

    /// The next smaller step; below the smallest step the page fits the pane.
    pub fn zoomed_out(self) -> Self {
        match self.percent() {
            None => Self::Page,
            Some(current) => STEPS
                .iter()
                .rev()
                .find(|step| **step < current)
                .map_or(Self::Page, |step| Self::Percent(*step)),
        }
    }

    /// Toggles between the two fit modes, and from a percent back to fit width.
    pub fn other(self) -> Self {
        match self {
            Self::Width => Self::Page,
            Self::Page | Self::Percent(_) => Self::Width,
        }
    }

    pub fn can_zoom_in(self) -> bool {
        self.zoomed_in() != self
    }

    pub fn can_zoom_out(self) -> bool {
        self != Self::Page
    }

    fn percent(self) -> Option<u16> {
        match self {
            Self::Percent(percent) => Some(percent),
            Self::Page | Self::Width => None,
        }
    }
}

/// One drawn page laid out for `zoom`. `id` must be unique on screen: it keeps each pane's
/// scroll position separate.
pub fn page_view(id: impl Into<ElementId>, image: Arc<RenderImage>, zoom: Zoom) -> AnyElement {
    let frame = div().flex_1().min_h_0().w_full().p_2();
    match zoom {
        Zoom::Page => frame
            .overflow_hidden()
            .child(img(image).object_fit(ObjectFit::Contain).size_full())
            .into_any_element(),
        Zoom::Width => frame
            .id(id)
            .overflow_y_scroll()
            .child(img(image).w_full())
            .into_any_element(),
        Zoom::Percent(_) => frame
            .id(id)
            .overflow_scroll()
            .child(img(image).w(px(zoom.width() as f32 / PIXELS_PER_POINT)))
            .into_any_element(),
    }
}

/// Receives the zoom level a control chose.
pub type ZoomHandler = Rc<dyn Fn(Zoom, &mut Window, &mut App)>;

/// `−`, the level (click to switch fit mode) and `+`.
pub fn zoom_controls(id: &str, zoom: Zoom, on_zoom: ZoomHandler) -> AnyElement {
    let button = |suffix: &str, label: SharedString, target: Zoom, enabled: bool| {
        let on_zoom = Rc::clone(&on_zoom);
        Button::new(SharedString::from(format!("{id}-{suffix}")), label)
            .size(ControlSize::Sm)
            .disabled(!enabled)
            .on_click(move |_, window, cx| on_zoom(target, window, cx))
    };
    div()
        .flex()
        .items_center()
        .gap_1()
        .child(button(
            "out",
            "−".into(),
            zoom.zoomed_out(),
            zoom.can_zoom_out(),
        ))
        .child(button("level", zoom.label(), zoom.other(), true))
        .child(button(
            "in",
            "+".into(),
            zoom.zoomed_in(),
            zoom.can_zoom_in(),
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::Zoom;

    #[test]
    fn steps_walk_from_fit_through_percent_and_back() {
        assert_eq!(Zoom::default(), Zoom::Width);
        assert_eq!(Zoom::Width.zoomed_in(), Zoom::Percent(100));
        assert_eq!(Zoom::Percent(100).zoomed_in(), Zoom::Percent(125));
        assert_eq!(Zoom::Percent(400).zoomed_in(), Zoom::Percent(400));
        assert!(!Zoom::Percent(400).can_zoom_in());
        assert_eq!(Zoom::Percent(50).zoomed_out(), Zoom::Page);
        assert_eq!(Zoom::Width.zoomed_out(), Zoom::Page);
        assert!(!Zoom::Page.can_zoom_out());
        assert_eq!(Zoom::Page.other(), Zoom::Width);
        assert_eq!(Zoom::Percent(200).other(), Zoom::Width);
    }

    #[test]
    fn larger_levels_draw_wider_bitmaps() {
        assert!(Zoom::Percent(400).width() > Zoom::Percent(100).width());
        assert_eq!(Zoom::Percent(100).width(), 1600);
        assert_eq!(Zoom::Page.width(), Zoom::Width.width());
    }
}
