//! Opens the NSH Documents reader (`sjel_gpui_ui::nsh_documents`) in a window of its own, drawn
//! by GPUI Fast (`docs/plans/gpui.md` §11, stage 3). `SJEL_NSH_TRIAL=1` asks for it at launch.

use std::sync::Arc;

use gpui_kit::App;
use nsh_platform_gpui::{open_app_window, set_ui_sets, AppOptions, AppSource, Root};
use sjel_gpui_core::device::Source;
use sjel_gpui_core::shell::Shell;
use sjel_gpui_ui::nsh_documents::{connect, DocumentsScreen};

pub fn open(cx: &mut App) {
    set_ui_sets(vec![nsh_base::registry()]);
    let endpoint = &sjel_gpui_core::environment::current().endpoint;
    if let Ok(shell) = Shell::new(endpoint) {
        connect(Source::Shell(Arc::new(shell)));
    }
    let source = AppSource::Rust(Root::of::<DocumentsScreen>());
    if let Err(error) = open_app_window(source, AppOptions::default(), cx) {
        eprintln!("sjel-gpui: the NSH trial window did not open: {error}");
    }
}
