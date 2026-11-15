use super::*;
use serde_json::json;
use sjel_gpui_core::{record::registry::Binding, shell::ShellError};
use std::collections::BTreeMap;

fn fixture() -> View {
    View {
        registry: serde_json::from_value(json!({"capability": "fixture", "objects": [{
            "id": "fixture.widget", "label": "Widgets",
            "schema": {"type": "object", "properties": {
                "id": {"type": "string"}, "label": {"type": "string"},
                "classification": {"type": "string"}, "quantity": {"type": "integer"}}},
            "identity": ["id"], "title": "label", "inactive_property": null,
            "classification": {"field": "classification", "derivation": "fixture", "inheritance": "all_properties"},
            "reads": [{"id": "list", "method": "GET", "path": "/api/widgets", "member": "widgets", "many": true, "parameters": []}],
            "links": [], "actions": []
        }]})).unwrap(),
        binding: Binding { object: "fixture.widget".into(), read: "list".into(), parameters: BTreeMap::new(), equals: BTreeMap::new() },
        rows: vec![json!({"id": "a", "label": "<script>literal text</script>", "classification": "c2", "quantity": 7})],
    }
}

#[gpui::test]
fn registry_rows_open_properties_and_back_by_pointer(cx: &mut gpui::TestAppContext) {
    let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| RegistryPage {
        repo: Repo::new(Err(ShellError::NotSetUp)),
        focus: cx.focus_handle(),
        capability: "fixture".into(),
        data: Remote {
            value: Some(fixture()),
            ..Remote::default()
        },
        selected: None,
        history: Vec::new(),
    });
    let row = cx
        .debug_bounds("table-row fixture.widget:[\"a\"]")
        .expect("registry identity row");
    cx.simulate_click(row.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.update(|window, cx| {
        assert!(
            page.read(cx).focus.is_focused(window),
            "property views retain page focus"
        );
    });
    page.read_with(cx, |page, _| {
        assert_eq!(
            page.selected.as_ref().unwrap()["label"],
            "<script>literal text</script>"
        );
        assert_eq!(page.selected.as_ref().unwrap()["quantity"], 7);
    });
    let back = cx
        .debug_bounds("registry-back-control")
        .expect("Back control");
    cx.simulate_click(back.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    page.read_with(cx, |page, _| assert!(page.selected.is_none()));
    assert!(cx
        .debug_bounds("table-row fixture.widget:[\"a\"]")
        .is_some());
}

#[gpui::test]
fn loading_rows_refuse_navigation(cx: &mut gpui::TestAppContext) {
    let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| RegistryPage {
        repo: Repo::new(Err(ShellError::NotSetUp)),
        focus: cx.focus_handle(),
        capability: "fixture".into(),
        data: Remote {
            value: Some(fixture()),
            loading: true,
            ..Remote::default()
        },
        selected: None,
        history: Vec::new(),
    });
    let row = cx.debug_bounds("table-row fixture.widget:[\"a\"]").unwrap();
    cx.simulate_click(row.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    page.read_with(cx, |page, _| assert!(page.selected.is_none()));
}

#[gpui::test]
fn another_object_starts_with_its_own_title_column(cx: &mut gpui::TestAppContext) {
    let (page, cx) = crate::test_support::mount_root(cx, None, |_, cx| {
        let mut view = fixture();
        view.rows = vec![
            json!({"id": "z", "label": "Alpha", "classification": "c2", "quantity": 1}),
            json!({"id": "a", "label": "Beta", "classification": "c2", "quantity": 2}),
        ];
        RegistryPage {
            repo: Repo::new(Err(ShellError::NotSetUp)),
            focus: cx.focus_handle(),
            capability: "fixture".into(),
            data: Remote {
                value: Some(view),
                ..Remote::default()
            },
            selected: None,
            history: Vec::new(),
        }
    });
    page.update(cx, |page, cx| {
        let view = page.data.value.as_mut().unwrap();
        view.registry.objects[0].id = "fixture.part".into();
        view.registry.objects[0].title = "id".into();
        view.binding.object = "fixture.part".into();
        cx.notify();
    });
    cx.run_until_parked();
    let first = cx.debug_bounds("table-row fixture.part:[\"z\"]").unwrap();
    cx.simulate_click(
        first.origin + gpui::point(px(20.), px(-10.)),
        gpui::Modifiers::none(),
    );
    cx.run_until_parked();
    let a = cx.debug_bounds("table-row fixture.part:[\"a\"]").unwrap();
    let z = cx.debug_bounds("table-row fixture.part:[\"z\"]").unwrap();
    assert!(
        a.top() < z.top(),
        "the first header sorts this object's title, id"
    );
}

#[gpui::test]
fn long_headers_keep_their_columns_in_a_narrow_view(cx: &mut gpui::TestAppContext) {
    let (_, cx) =
        crate::test_support::mount_root(cx, Some(gpui::size(px(500.), px(500.))), |_, cx| {
            let mut view = fixture();
            view.registry.objects[0].schema["properties"]["classification_rationale"] =
                json!({"type": "string"});
            view.rows[0]["classification_rationale"] = json!("A folder floor");
            RegistryPage {
                repo: Repo::new(Err(ShellError::NotSetUp)),
                focus: cx.focus_handle(),
                capability: "fixture".into(),
                data: Remote {
                    value: Some(view),
                    ..Remote::default()
                },
                selected: None,
                history: Vec::new(),
            }
        });
    let row = cx.debug_bounds("table-row fixture.widget:[\"a\"]").unwrap();
    assert!(
        row.size.width > px(500.),
        "columns scroll instead of squeezing their headings"
    );
}
