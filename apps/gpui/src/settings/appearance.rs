//! Appearance settings: Theme mode, accent colors, liquid glass, opacities, and terminal styling.

use gpui_kit::component::switch::Switch;
use gpui_kit::{AnyElement, Context, SharedString};
use sjel_gpui_ui::theme::Palette;
use sjel_gpui_ui::theme::{
    AppearanceSettings, BackgroundTone, GlassStyle, TerminalFont, ThemeMode, WindowOpacity,
    TERMINAL_FONT_SIZES,
};

use crate::workspace::Workspace;
use super::{accent_swatches, group, row, segmented};

pub(super) fn appearance(
    settings: AppearanceSettings,
    palette: Palette,
    cx: &mut Context<Workspace>,
) -> Vec<AnyElement> {
    let theme = segmented(
        "theme-mode",
        ThemeMode::ALL.map(|mode| (mode.label(), settings.theme_mode == mode)),
        cx,
        move |ix| AppearanceSettings {
            theme_mode: ThemeMode::ALL[ix],
            ..settings
        },
    );
    let opacity = segmented(
        "window-opacity",
        WindowOpacity::ALL.map(|o| (o.short_label(), settings.opacity == o)),
        cx,
        move |ix| AppearanceSettings {
            opacity: WindowOpacity::ALL[ix],
            ..settings
        },
    );
    let sidebar_opacity = segmented(
        "sidebar-opacity",
        WindowOpacity::ALL.map(|o| (o.short_label(), settings.sidebar_opacity == o)),
        cx,
        move |ix| AppearanceSettings {
            sidebar_opacity: WindowOpacity::ALL[ix],
            ..settings
        },
    );
    let panel_opacity = segmented(
        "panel-opacity",
        WindowOpacity::ALL.map(|o| (o.short_label(), settings.panel_opacity == o)),
        cx,
        move |ix| AppearanceSettings {
            panel_opacity: WindowOpacity::ALL[ix],
            ..settings
        },
    );
    let tone = segmented(
        "background-tone",
        BackgroundTone::ALL.map(|t| (t.label(), settings.tone == t)),
        cx,
        move |ix| AppearanceSettings {
            tone: BackgroundTone::ALL[ix],
            ..settings
        },
    );
    let terminal_tone = segmented(
        "terminal-tone",
        BackgroundTone::ALL.map(|t| (t.label(), settings.terminal_tone == t)),
        cx,
        move |ix| AppearanceSettings {
            terminal_tone: BackgroundTone::ALL[ix],
            ..settings
        },
    );
    let terminal_opacity = segmented(
        "terminal-opacity",
        WindowOpacity::ALL.map(|o| (o.short_label(), settings.terminal_opacity == o)),
        cx,
        move |ix| AppearanceSettings {
            terminal_opacity: WindowOpacity::ALL[ix],
            ..settings
        },
    );
    let fonts = TerminalFont::available(cx);
    let terminal_font = segmented(
        "terminal-font",
        fonts
            .iter()
            .map(|font| (font.label(), settings.terminal_font == *font))
            .collect::<Vec<_>>(),
        cx,
        move |ix| AppearanceSettings {
            terminal_font: fonts[ix],
            ..settings
        },
    );
    let terminal_font_size = segmented(
        "terminal-font-size",
        TERMINAL_FONT_SIZES.map(|size| {
            (
                SharedString::from(size.to_string()),
                settings.terminal_font_size == size,
            )
        }),
        cx,
        move |ix| AppearanceSettings {
            terminal_font_size: TERMINAL_FONT_SIZES[ix],
            ..settings
        },
    );
    let glass = Switch::new("liquid-glass")
        .checked(settings.glass == GlassStyle::LiquidGlass)
        .on_click(cx.listener(move |view, on: &bool, _, cx| {
            let glass = if *on {
                GlassStyle::LiquidGlass
            } else {
                GlassStyle::Flat
            };
            view.set_appearance(AppearanceSettings { glass, ..settings }, cx);
        }));
    vec![
        group(
            "THEME",
            vec![
                row(
                    "Mode",
                    Some("Follow macOS, or keep Sjel light or dark."),
                    theme,
                    palette,
                ),
                row(
                    "Accent colour",
                    Some("Selection, focus rings and primary buttons."),
                    accent_swatches(settings, cx),
                    palette,
                ),
            ],
            palette,
        ),
        group(
            "WINDOW",
            vec![
                row(
                    "Liquid Glass sidebar",
                    Some("Draws the sidebar on native glass. Off draws it solid."),
                    glass,
                    palette,
                ),
                row(
                    "Canvas opacity",
                    Some("How much of the desktop shows through the main window."),
                    opacity,
                    palette,
                ),
                row(
                    "Left sidebar opacity",
                    Some("Set the navigation surface separately from the canvas."),
                    sidebar_opacity,
                    palette,
                ),
                row(
                    "More panel opacity",
                    Some("Set the right-side panel surface separately."),
                    panel_opacity,
                    palette,
                ),
                row(
                    "Background tone",
                    Some("Tints the window. System uses the macOS colour."),
                    tone,
                    palette,
                ),
            ],
            palette,
        ),
        group(
            "TERMINAL",
            vec![
                row(
                    "Font",
                    Some("Auto uses an installed Nerd Font, so prompt and ls icons draw whole."),
                    terminal_font,
                    palette,
                ),
                row("Font size", Some("Points."), terminal_font_size, palette),
                row(
                    "Background",
                    Some("System is Monokai's background. The colours are Monokai."),
                    terminal_tone,
                    palette,
                ),
                row(
                    "Background opacity",
                    Some("How much of the window shows through the grid."),
                    terminal_opacity,
                    palette,
                ),
            ],
            palette,
        ),
    ]
}
