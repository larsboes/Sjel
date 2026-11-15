//! Object discovery shared by capability contracts and derived views.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A capability's object declarations. Property types come from serialized wire schemas.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub capability: String,
    pub objects: Vec<Object>,
}

/// A read projection with identity and object-wide classification inheritance.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub id: String,
    pub label: String,
    pub schema: Value,
    pub identity: Vec<String>,
    pub title: String,
    pub inactive_property: Option<String>,
    pub classification: Classification,
    pub reads: Vec<Read>,
    pub links: Vec<Link>,
    pub actions: Vec<Action>,
}

/// Every property inherits the object's class; no property provenance is asserted.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Classification {
    pub field: String,
    pub derivation: String,
    pub inheritance: Inheritance,
}

/// Classification inheritance supported by these read projections.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Inheritance {
    AllProperties,
}

/// An approved read and the response member containing this projection.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Read {
    pub id: String,
    pub method: String,
    pub path: String,
    pub member: String,
    pub many: bool,
    pub parameters: Vec<String>,
    #[serde(default)]
    pub fixed_parameters: std::collections::BTreeMap<String, String>,
}

/// A relation resolved through a target object's approved read.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub label: String,
    pub target: String,
    pub read: String,
    pub parameters: std::collections::BTreeMap<String, String>,
    pub equals: std::collections::BTreeMap<String, String>,
}

/// Discovery only: views cannot execute actions. Existing capability policy owns proposals.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub method: String,
    pub path: String,
    pub governance: String,
}

impl Registry {
    /// Reject invalid property, relation and route bindings before publishing discovery.
    ///
    /// # Errors
    /// Returns the object or link that fails validation.
    pub fn validate(&self, manifest: &Value, get_writes: &[(&str, &str)]) -> Result<(), String> {
        let routes = manifest["routes"]
            .as_array()
            .ok_or("registry: missing routes")?;
        for (index, object) in self.objects.iter().enumerate() {
            let fail = |message: &str| format!("{}: {message}", object.id);
            if self.objects[..index].iter().any(|o| o.id == object.id) {
                return Err(fail("duplicate object"));
            }
            let props = object.schema["properties"]
                .as_object()
                .ok_or_else(|| fail("missing properties"))?;
            if object.identity.is_empty()
                || object
                    .identity
                    .iter()
                    .chain([&object.title, &object.classification.field])
                    .any(|p| !props.contains_key(p))
            {
                return Err(fail("unknown identity, title or classification property"));
            }
            if object.inactive_property.as_ref().is_some_and(|property| {
                props
                    .get(property)
                    .is_none_or(|schema| schema["type"] != "boolean")
            }) {
                return Err(fail("inactive property must name a boolean"));
            }
            for (at, read) in object.reads.iter().enumerate() {
                if object.reads[..at].iter().any(|r| r.id == read.id) {
                    return Err(fail("duplicate read"));
                }
                if read
                    .parameters
                    .iter()
                    .any(|parameter| read.fixed_parameters.contains_key(parameter))
                {
                    return Err(fail("fixed parameter cannot be rebound"));
                }
                if read.method != "GET"
                    || get_writes.contains(&(read.method.as_str(), read.path.as_str()))
                {
                    return Err(fail("write route cannot bind a view"));
                }
                let route = routes
                    .iter()
                    .find(|r| r["method"] == read.method && r["path"] == read.path)
                    .ok_or_else(|| fail("undeclared read route"))?;
                let response = &route["response_schema"];
                let member = &response["properties"][&read.member];
                let projection = if read.many {
                    if member["type"] != "array" {
                        return Err(fail("response member is not an array"));
                    }
                    &member["items"]
                } else {
                    member
                };
                let projection = match projection["$ref"].as_str() {
                    Some(reference) => response
                        .pointer(
                            reference
                                .strip_prefix('#')
                                .ok_or_else(|| fail("external schema reference"))?,
                        )
                        .ok_or_else(|| fail("unresolved response projection"))?,
                    None => projection,
                };
                if projection["properties"] != object.schema["properties"]
                    || projection["required"] != object.schema["required"]
                {
                    return Err(fail("response projection differs from object schema"));
                }
            }
            for link in &object.links {
                let target = self
                    .objects
                    .iter()
                    .find(|o| o.id == link.target)
                    .ok_or_else(|| fail("unknown link target"))?;
                let read = target
                    .reads
                    .iter()
                    .find(|r| r.id == link.read)
                    .ok_or_else(|| fail("unknown linked read"))?;
                if link.parameters.len() != read.parameters.len()
                    || link.parameters.keys().any(|p| !read.parameters.contains(p))
                    || link
                        .parameters
                        .values()
                        .chain(link.equals.values())
                        .any(|p| !props.contains_key(p))
                    || link
                        .equals
                        .keys()
                        .any(|p| target.schema["properties"].get(p).is_none())
                {
                    return Err(fail("invalid link binding"));
                }
            }
            for action in &object.actions {
                if action.governance != "capability-policy"
                    || action.method == "GET"
                    || !routes.iter().any(|r| {
                        r["method"] == action.method
                            && r["path"] == action.path
                            && r["request_schema"].is_object()
                    })
                {
                    return Err(fail("invalid governed action"));
                }
            }
        }
        Ok(())
    }
}
