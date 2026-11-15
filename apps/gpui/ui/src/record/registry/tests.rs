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
    let (page, cx) = crate::test_support::mount_root(cx, None, |_, _| RegistryPage {
        repo: Repo::new(Err(ShellError::NotSetUp)),
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
    let (page, cx) = crate::test_support::mount_root(cx, None, |_, _| RegistryPage {
        repo: Repo::new(Err(ShellError::NotSetUp)),
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
