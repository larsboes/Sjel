use super::*;
use crate::data::Repo;
use std::{cell::RefCell, rc::Rc};

#[gpui::test]
fn pending_review_renders_and_typing_remains_an_explicit_unsaved_choice(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_kit::init);
    let slot = Rc::new(RefCell::new(None));
    let found = slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let view = cx.new(|cx| {
            let mut page = MediaPage::new(Repo::new(Err(ShellError::NotSetUp)));
            let detail: ContentDetail = serde_json::from_value(serde_json::json!({
                "proposal":{"id":1,"sha256":"a".repeat(64),"fingerprint":"b".repeat(64),
                    "kind":"image","engine":"synthetic-fixture","revision":1,"state":"pending",
                    "tags":[],"collections":[],"created_at":"fixture"},
                "os":"Synthetic OS","filenames":["fixture.png"],
                "frames":[{"requestedSeconds":null,"seconds":null,"previewPng":"",
                    "labels":[{"identifier":"raw_candidate_not_a_tag","score":0.8}]}]
            }))
            .unwrap();
            page.content.queue.value = Some(ContentPage {
                items: vec![detail.proposal.clone()],
                next_after: None,
            });
            page.content.detail.value = Some(detail);
            page.content.previews = vec![Arc::new(Image::from_bytes(
                ImageFormat::Png,
                include_bytes!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../../crates/extraction/eval/fixtures/en-prose.png"
                ))
                .to_vec(),
            ))];
            let tags = cx.new(|cx| InputState::new(window, cx));
            let collections = cx.new(|cx| InputState::new(window, cx));
            tags.update(cx, |input, cx| input.focus(window, cx));
            page.content.form = Some(ReviewForm {
                id: 1,
                tags,
                collections,
                _subscriptions: vec![],
            });
            page
        });
        found.replace(Some(view.clone()));
        gpui_kit::component::Root::new(view, window, cx)
    });
    cx.run_until_parked();
    let page = slot.take().unwrap();
    page.read_with(cx, |page, cx| {
        assert!(page
            .content
            .form
            .as_ref()
            .unwrap()
            .tags
            .read(cx)
            .value()
            .is_empty());
        assert!(page
            .content
            .detail
            .value
            .as_ref()
            .unwrap()
            .proposal
            .tags
            .is_empty());
    });
    cx.simulate_keystrokes("f a m i l y");
    cx.run_until_parked();
    page.update(cx, |page, cx| {
        assert_eq!(
            page.content
                .form
                .as_ref()
                .unwrap()
                .tags
                .read(cx)
                .value()
                .as_str(),
            "family"
        );
        let proposal = &page.content.detail.value.as_ref().unwrap().proposal;
        assert_eq!(proposal.state, ContentState::Pending);
        assert!(proposal.tags.is_empty());
        page.content.queue.error = Some(ShellError::Unreachable);
        page.decide_content(ContentAction::Accept, cx);
        assert!(
            page.content.message.is_none(),
            "stale evidence cannot submit"
        );
        page.content.queue.error = None;
        page.decide_content(ContentAction::Accept, cx);
        assert!(page.content.message.as_ref().unwrap().0);
        assert!(!page.content.busy);
        assert_eq!(
            page.content
                .form
                .as_ref()
                .unwrap()
                .tags
                .read(cx)
                .value()
                .as_str(),
            "family"
        );
        assert_eq!(
            page.content.detail.value.as_ref().unwrap().proposal.state,
            ContentState::Pending
        );
    });
}
