#![cfg(feature = "nsh")]

use nsh_core::element::Root;
use nsh_core::platform::Appearance;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;
use sjel_gpui_ui::nsh_documents::{self, NshHostScreen, NshPage};

#[test]
fn today_documents_and_mail_render_and_switch_in_the_phone_host() {
    nsh_documents::set_close(|| {});
    for appearance in [Appearance::Light, Appearance::Dark] {
        nsh_documents::set_nsh_page(NshPage::Documents);
        let mut app = Headless::start(EngineConfig {
            ui_sets: vec![nsh_base::registry()],
            ..EngineConfig::new(Root::of::<NshHostScreen>().appearance(appearance))
        })
        .expect("the phone's NSH root starts");

        app.wait_for("text:Documents")
            .expect("Documents renders its heading");
        assert_readable_heading(&app, "Documents", appearance);
        app.press("text:Mail").expect("the Mail tab responds");
        app.wait_for("text:Mail").expect("Mail renders its heading");
        assert_readable_heading(&app, "Mail", appearance);
        app.press("text:Docs").expect("the Docs tab responds");
        app.wait_for("text:Documents")
            .expect("Documents renders again after switching back");
        app.press("text:Today").expect("the Today tab responds");
        app.wait_for("text:TODAY")
            .expect("Today renders its heading");
    }
}

fn assert_readable_heading(app: &Headless, heading: &str, appearance: Appearance) {
    let color = app
        .screen()
        .span_style(&format!("text:/^{heading}$/"))
        .unwrap()
        .color
        .0;
    let brightness = ((color >> 24) + ((color >> 16) & 255) + ((color >> 8) & 255)) / 3;
    match appearance {
        Appearance::Dark => assert!(
            brightness > 127,
            "dark appearance needs readable light text: {color:08x}"
        ),
        Appearance::Light => assert!(
            brightness < 128,
            "light appearance needs readable dark text: {color:08x}"
        ),
        Appearance::System => panic!("the test must choose a concrete appearance"),
    }
}
