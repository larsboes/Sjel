//! TypeScript read contracts derived from serialized JSON schemas.

use serde_json::Value;

/// Render one named contract, resolving local references within its root schema.
///
/// # Errors
/// Rejects unsupported shapes, invalid names and unresolved or recursive references.
pub fn declaration(schema: &Value) -> Result<String, String> {
    let name = schema["title"].as_str().ok_or("schema has no title")?;
    if name.is_empty()
        || !name
            .bytes()
            .enumerate()
            .all(|(i, c)| c.is_ascii_alphabetic() || c == b'_' || (i > 0 && c.is_ascii_digit()))
    {
        return Err(format!("invalid TypeScript type name: {name}"));
    }
    Ok(format!(
        "export type {name} = {};\n",
        render(schema, schema, &mut Vec::new())?
    ))
}

fn render(schema: &Value, root: &Value, refs: &mut Vec<String>) -> Result<String, String> {
    for keyword in [
        "allOf",
        "anyOf",
        "oneOf",
        "not",
        "if",
        "then",
        "else",
        "prefixItems",
        "patternProperties",
        "const",
    ] {
        if schema.get(keyword).is_some() {
            return Err(format!("unsupported TypeScript schema keyword: {keyword}"));
        }
    }
    if schema
        .get("additionalProperties")
        .is_some_and(Value::is_object)
    {
        return Err("typed additionalProperties is unsupported".into());
    }
    if let Some(reference) = schema["$ref"].as_str() {
        if refs.iter().any(|seen| seen == reference) {
            return Err(format!("recursive schema reference: {reference}"));
        }
        let target = reference
            .strip_prefix('#')
            .and_then(|path| root.pointer(path))
            .ok_or_else(|| format!("unresolved local schema reference: {reference}"))?;
        refs.push(reference.to_owned());
        let result = render(target, root, refs);
        refs.pop();
        return result;
    }
    if let Some(values) = schema["enum"].as_array() {
        if values.is_empty() || values.iter().any(|v| v.is_object() || v.is_array()) {
            return Err("enum must contain scalar values".into());
        }
        return Ok(values
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join(" | "));
    }
    if let Some(types) = schema["type"].as_array() {
        let mut alternatives = Vec::new();
        for kind in types {
            let mut branch = schema.clone();
            branch["type"] = kind.clone();
            alternatives.push(render(&branch, root, refs)?);
        }
        if alternatives.is_empty() {
            return Err("empty type union".into());
        }
        return Ok(alternatives.join(" | "));
    }
    match schema["type"].as_str() {
        Some("string") => Ok("string".into()),
        Some("boolean") => Ok("boolean".into()),
        Some("integer" | "number") => Ok("number".into()),
        Some("null") => Ok("null".into()),
        Some("array") => Ok(format!("Array<{}>", render(&schema["items"], root, refs)?)),
        Some("object") => {
            let props = schema["properties"]
                .as_object()
                .ok_or("object has no properties")?;
            let mut fields = Vec::with_capacity(props.len());
            for (key, value) in props {
                let required = schema["required"]
                    .as_array()
                    .is_some_and(|keys| keys.iter().any(|k| k == key));
                fields.push(format!(
                    "  {}{}: {};",
                    Value::String(key.clone()),
                    if required { "" } else { "?" },
                    render(value, root, refs)?.replace('\n', "\n  ")
                ));
            }
            Ok(format!("{{\n{}\n}}", fields.join("\n")))
        }
        _ => Err(format!("unsupported TypeScript schema: {schema}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_optional_nullable_enum_and_array_shapes() {
        let schema = json!({"title": "Read", "type": "object", "required": ["due", "items"],
            "properties": {"due": {"type": ["string", "null"]},
                "body": {"type": "string"},
                "items": {"type": "array", "items": {"$ref": "#/$defs/Kind"}}},
            "$defs": {"Kind": {"type": "string", "enum": ["note", "task"]}}});
        let output = declaration(&schema).unwrap();
        assert!(output.contains("\"body\"?: string;"));
        assert!(output.contains("\"due\": string | null;"));
        assert!(output.contains("\"items\": Array<\"note\" | \"task\">;"));
    }

    #[test]
    fn refuses_unsupported_or_recursive_shapes() {
        for schema in [
            json!({"title": "Bad", "anyOf": [{"type": "string"}]}),
            json!({"title": "Bad", "type": "string", "oneOf": [{"type": "string"}]}),
            json!({"title": "Bad", "type": "object", "properties": {}, "additionalProperties": {"type": "string"}}),
            json!({"title": "Bad", "$ref": "https://example.org/schema"}),
            json!({"title": "Bad", "$ref": "#/$defs/Loop", "$defs": {"Loop": {"$ref": "#/$defs/Loop"}}}),
            json!({"title": "Bad; injected", "type": "string"}),
        ] {
            assert!(declaration(&schema).is_err());
        }
    }
}
