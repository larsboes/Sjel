#![cfg(feature = "nsh")]

use nsh_core::element::Root;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;
use sjel_gpui_ui::nsh_documents::{self, NshHostScreen, NshPage};

#[test]
fn documents_and_mail_render_and_switch_in_the_phone_host() {
    nsh_documents::set_close(|| {});
    nsh_documents::set_nsh_page(NshPage::Documents);
    let mut app = Headless::start(EngineConfig {
        ui_sets: vec![nsh_base::registry()],
        ..EngineConfig::new(Root::of::<NshHostScreen>())
    })
    .expect("the phone's NSH root starts");

    app.wait_for("text:Documents")
        .expect("Documents renders its heading");
    app.press("text:Mail").expect("the Mail tab responds");
    app.wait_for("text:Mail").expect("Mail renders its heading");
    app.press("text:Documents")
        .expect("the Documents tab responds");
    app.wait_for("text:Documents")
        .expect("Documents renders again after switching back");
}
