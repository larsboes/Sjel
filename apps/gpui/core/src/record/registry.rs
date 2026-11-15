//! Read-only navigation through a capability's published object declarations.

use std::collections::BTreeMap;

use reqwest::Method;
use route_manifest::registry::Registry;
use serde_json::Value;

use crate::shell::{Shell, ShellError, ShellResult};

/// A binding to a declared read, with relation filters applied after the response.
#[derive(Clone, Debug)]
pub struct Binding {
    pub object: String,
    pub read: String,
    pub parameters: BTreeMap<String, String>,
    pub equals: BTreeMap<String, Value>,
}

/// The registry and one read's objects. No action execution is exposed.
#[derive(Clone, Debug)]
pub struct View {
    pub registry: Registry,
    pub binding: Binding,
    pub rows: Vec<Value>,
}

fn refused(message: impl Into<String>) -> ShellError {
    ShellError::Refused(400, Some(message.into()))
}

/// Validate discovery before any object route can be called.
///
/// # Errors
/// Names the invalid object node or the invalid capability proxy path.
pub fn registry(manifest: &Value, capability: &str) -> ShellResult<Registry> {
    if capability.is_empty()
        || !capability
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        || manifest["capability"] != capability
    {
        return Err(refused("registry: invalid capability"));
    }
    let objects = manifest["objects"]
        .as_array()
        .ok_or_else(|| refused("/objects: expected array"))?;
    let mut parsed = Vec::with_capacity(objects.len());
    for (i, object) in objects.iter().enumerate() {
        parsed.push(
            serde_json::from_value(object.clone())
                .map_err(|error| refused(format!("/objects/{i}: {error}")))?,
        );
    }
    let registry = Registry {
        capability: capability.into(),
        objects: parsed,
    };
    registry.validate(manifest, &[]).map_err(refused)?;
    for object in &registry.objects {
        for read in &object.reads {
            if !read.path.starts_with("/api/")
                || read.path.contains(['?', '#', '%', '\\'])
                || read.path.split('/').any(|part| part == "." || part == "..")
            {
                return Err(refused(format!("{}: invalid read path", object.id)));
            }
        }
    }
    Ok(registry)
}

/// Load the first unparameterized object list from validated capability discovery.
///
/// # Errors
/// Discovery, transport, or binding validation fails.
pub async fn load(shell: &Shell, capability: &str) -> ShellResult<View> {
    if capability.is_empty()
        || !capability
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
    {
        return Err(refused("registry: invalid capability"));
    }
    let manifest: Value = shell
        .json(Method::GET, &format!("/{capability}/routes"), None, 10)
        .await?;
    let registry = registry(&manifest, capability)?;
    let (object, reader) = registry
        .objects
        .iter()
        .find_map(|object| {
            object
                .reads
                .iter()
                .find(|read| read.many && read.parameters.is_empty())
                .map(|read| (object, read))
        })
        .ok_or_else(|| refused("/objects: no unparameterized list read"))?;
    let binding = Binding {
        object: object.id.clone(),
        read: reader.id.clone(),
        parameters: BTreeMap::new(),
        equals: BTreeMap::new(),
    };
    read(shell, registry, binding).await
}

impl View {
    /// The current validated object projection.
    pub fn object(&self) -> &route_manifest::registry::Object {
        self.registry
            .objects
            .iter()
            .find(|object| object.id == self.binding.object)
            .expect("read validates the bound object")
    }

    /// Bind a declared relation to one source row without accepting an arbitrary route.
    ///
    /// # Errors
    /// The link or a source property is absent.
    pub fn link(&self, row: &Value, index: usize) -> ShellResult<Binding> {
        let link = self
            .object()
            .links
            .get(index)
            .ok_or_else(|| refused("unknown link"))?;
        let mut parameters = BTreeMap::new();
        for (parameter, field) in &link.parameters {
            let value = row[field]
                .as_str()
                .ok_or_else(|| refused(format!("{field}: link parameter must be text")))?;
            parameters.insert(parameter.clone(), value.into());
        }
        let mut equals = BTreeMap::new();
        for (target, source) in &link.equals {
            equals.insert(
                target.clone(),
                row.get(source)
                    .cloned()
                    .ok_or_else(|| refused(format!("{source}: missing relation property")))?,
            );
        }
        Ok(Binding {
            object: link.target.clone(),
            read: link.read.clone(),
            parameters,
            equals,
        })
    }
}

/// Fetch only a declared GET projection; fixed parameters cannot be overridden.
///
/// # Errors
/// Rejects an unknown binding, malformed response, or a transport failure.
pub async fn read(shell: &Shell, registry: Registry, binding: Binding) -> ShellResult<View> {
    let (path, member, many) = request(&registry, &binding)?;
    let response: Value = shell.json(Method::GET, &path, None, 10).await?;
    project(registry, binding, &response, &member, many)
}

fn request(registry: &Registry, binding: &Binding) -> ShellResult<(String, String, bool)> {
    let object = registry
        .objects
        .iter()
        .find(|object| object.id == binding.object)
        .ok_or_else(|| refused(format!("{}: unknown object", binding.object)))?;
    let read = object
        .reads
        .iter()
        .find(|read| read.id == binding.read && read.method == "GET")
        .ok_or_else(|| refused(format!("{}: unknown read", object.id)))?;
    if binding.parameters.len() != read.parameters.len()
        || binding
            .parameters
            .keys()
            .any(|key| !read.parameters.contains(key))
        || binding
            .equals
            .keys()
            .any(|key| object.schema["properties"].get(key).is_none())
    {
        return Err(refused(format!("{}: invalid read parameters", object.id)));
    }
    let mut query = reqwest::Url::parse("http://localhost").map_err(|_| ShellError::Unreadable)?;
    query.set_path(&format!("/{}{}", registry.capability, read.path));
    for (key, value) in binding.parameters.iter().chain(&read.fixed_parameters) {
        query.query_pairs_mut().append_pair(key, value);
    }
    Ok((
        format!(
            "{}{}",
            query.path(),
            query.query().map_or(String::new(), |q| format!("?{q}"))
        ),
        read.member.clone(),
        read.many,
    ))
}

fn project(
    registry: Registry,
    binding: Binding,
    response: &Value,
    member: &str,
    many: bool,
) -> ShellResult<View> {
    let object = registry
        .objects
        .iter()
        .find(|object| object.id == binding.object)
        .ok_or_else(|| refused(format!("{}: unknown object", binding.object)))?;
    let member = response
        .get(member)
        .ok_or_else(|| refused(format!("{}: missing response member {}", object.id, member)))?;
    let rows = if many {
        member
            .as_array()
            .cloned()
            .ok_or_else(|| refused(format!("{}: expected object array", object.id)))?
    } else {
        vec![member.clone()]
    };
    for (i, row) in rows.iter().enumerate() {
        if !row.is_object()
            || !row[&object.title].is_string()
            || object.identity.iter().any(|key| row.get(key).is_none())
            || !matches!(
                row[&object.classification.field].as_str(),
                Some("c0" | "c1" | "c2" | "c3")
            )
        {
            return Err(refused(format!(
                "{}/rows/{i}: missing identity, title or classification",
                object.id
            )));
        }
    }
    let rows = rows
        .into_iter()
        .filter(|row| {
            binding
                .equals
                .iter()
                .all(|(key, value)| row.get(key) == Some(value))
        })
        .collect();
    Ok(View {
        registry,
        binding,
        rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest() -> Value {
        let declaration = projects_api::registry::declaration();
        let mut routes = projects_api::registry::READ_RESPONSES
            .iter()
            .map(|r| {
                json!({
                    "method": r.method, "path": r.path, "response_schema": (r.schema)()
                })
            })
            .collect::<Vec<_>>();
        for action in &declaration.objects[1].actions {
            routes
                .push(json!({"method": action.method, "path": action.path, "request_schema": {}}));
        }
        json!({"capability": "projects", "objects": declaration.objects, "routes": routes})
    }

    fn view() -> View {
        View {
            registry: registry(&manifest(), "projects").unwrap(),
            binding: Binding {
                object: "projects.project".into(),
                read: "list".into(),
                parameters: BTreeMap::new(),
                equals: BTreeMap::new(),
            },
            rows: vec![],
        }
    }

    #[test]
    fn errors_name_the_invalid_node_and_refuse_write_bindings() {
        let mut invalid = manifest();
        invalid["objects"][0]["title"] = json!("missing");
        assert!(registry(&invalid, "projects")
            .unwrap_err()
            .to_string()
            .contains("projects.project"));
        let mut invalid = manifest();
        invalid["objects"][0]["reads"][0]["method"] = json!("POST");
        assert!(registry(&invalid, "projects")
            .unwrap_err()
            .to_string()
            .contains("write route"));
        let mut invalid = manifest();
        invalid["objects"][0]["unexpected"] = json!(true);
        assert!(registry(&invalid, "projects")
            .unwrap_err()
            .to_string()
            .contains("/objects/0"));
        assert!(registry(&manifest(), "../projects").is_err());
    }

    #[test]
    fn relation_preserves_archived_identity_and_fixed_parameters() {
        let view = view();
        let row = json!({"name": "A&B ?/研究", "archived": true});
        let binding = view.link(&row, 0).unwrap();
        assert_eq!(binding.equals["archived"], true);
        let (path, member, many) = request(&view.registry, &binding).unwrap();
        let url = reqwest::Url::parse(&format!("http://localhost{path}")).unwrap();
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query["name"], "A&B ?/研究");
        assert_eq!(query["content"], "false");
        assert_eq!(member, "records");
        assert!(many);
        let mut invalid = binding.clone();
        invalid.parameters.insert("content".into(), "true".into());
        assert!(request(&view.registry, &invalid).is_err());
        let response = json!({"records": [
            {"path": "Archive/A/n.md", "title": "Archived", "archived": true, "data_class": "c2"},
            {"path": "A/n.md", "title": "Live", "archived": false, "data_class": "c1"}
        ]});
        let projected = project(view.registry, binding, &response, &member, many).unwrap();
        assert_eq!(projected.rows.len(), 1);
        assert_eq!(projected.rows[0]["title"], "Archived");
        assert_eq!(projected.rows[0]["data_class"], "c2");
    }

    #[test]
    fn missing_class_is_an_error_rather_than_a_default() {
        let view = view();
        let response = json!({"projects": [{"name": "A", "archived": false}]});
        assert!(
            project(view.registry, view.binding, &response, "projects", true)
                .unwrap_err()
                .to_string()
                .contains("projects.project/rows/0")
        );
    }
}
