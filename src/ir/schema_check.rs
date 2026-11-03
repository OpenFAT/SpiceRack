//! Test only: JSON → the places it breaks `schema/circuit-ir.schema.json`.
//!
//! A JSON Schema 2020-12 validator for exactly the keywords that file uses
//! (json-schema.org/draft/2020-12, Core §8.2.3.1 `$ref`, §10.2.1 `oneOf`,
//! §10.3.1 `prefixItems`/`items`, §10.3.2 `properties`/
//! `additionalProperties`; Validation §6.1-§6.5). Any other keyword panics,
//! so a schema cannot pass by the checker skipping what it does not read.
//! `jsonschema` is in neither the nix shell nor the lockfile; this is the
//! ponytail size of one.
//!
//! Contents:
//! - `pub(super) fn errors(root: &Value, doc: &Value) -> Vec<String>`

use serde_json::Value;

/// The keywords that only annotate, or that `check` reads beside another.
const PASSIVE: [&str; 11] = [
    "$schema",
    "$id",
    "$defs",
    "title",
    "description",
    "default",
    "deprecated",
    "properties",
    "additionalProperties",
    "prefixItems",
    "items",
];

/// Every place `doc` breaks `root`, as `path: reason`; empty when it holds.
pub(super) fn errors(root: &Value, doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    check(root, root, doc, "$", 0, &mut out);
    out
}

// ---- private ----

fn check(root: &Value, s: &Value, v: &Value, at: &str, depth: u32, out: &mut Vec<String>) {
    // The IR nests a few levels; a `$ref` cycle with no data under it would not end.
    assert!(depth < 64, "schema recursion past 64 at {at}");
    let s = match s {
        Value::Bool(true) => return,
        Value::Bool(false) => return out.push(format!("{at}: no value is allowed here")),
        Value::Object(s) => s,
        other => panic!("schema at {at} is {other}, not an object or a boolean"),
    };
    for (k, kv) in s {
        match k.as_str() {
            "$ref" => {
                let name = kv
                    .as_str()
                    .and_then(|r| r.strip_prefix("#/$defs/"))
                    .unwrap_or_else(|| panic!("non-local $ref {kv} at {at}"));
                let def = root["$defs"]
                    .get(name)
                    .unwrap_or_else(|| panic!("$ref to missing $defs/{name} at {at}"));
                check(root, def, v, at, depth + 1, out);
            }
            "type" => {
                let names: Vec<&str> = match kv {
                    Value::String(t) => vec![t],
                    Value::Array(ts) => ts.iter().map(|t| t.as_str().expect("type name")).collect(),
                    other => panic!("type {other} at {at}"),
                };
                if !names.iter().any(|t| is_type(t, v)) {
                    out.push(format!("{at}: {v} is not {}", names.join(" or ")));
                }
            }
            "const" if v != kv => out.push(format!("{at}: {v} is not {kv}")),
            "enum" if !kv.as_array().expect("enum list").contains(v) => {
                out.push(format!("{at}: {v} is not one of {kv}"))
            }
            "required" => {
                for r in kv
                    .as_array()
                    .expect("required list")
                    .iter()
                    .filter_map(Value::as_str)
                {
                    if v.as_object().is_some_and(|o| !o.contains_key(r)) {
                        out.push(format!("{at}: missing {r}"));
                    }
                }
            }
            "minItems" | "maxItems" => {
                let (n, bound) = (
                    v.as_array().map_or(0, Vec::len) as u64,
                    kv.as_u64().expect("item count"),
                );
                let bad = v.is_array()
                    && if k == "minItems" {
                        n < bound
                    } else {
                        n > bound
                    };
                if bad {
                    out.push(format!("{at}: {n} items breaks {k} {bound}"));
                }
            }
            "minimum"
                if v.as_f64()
                    .is_some_and(|x| x < kv.as_f64().expect("minimum")) =>
            {
                out.push(format!("{at}: {v} < {kv}"))
            }
            "oneOf" => {
                let branches = kv.as_array().expect("oneOf list");
                let errs: Vec<Vec<String>> = branches
                    .iter()
                    .map(|b| errors_under(root, b, v, at, depth))
                    .collect();
                match errs.iter().filter(|e| e.is_empty()).count() {
                    1 => {}
                    0 => out.push(format!(
                        "{at}: matches no oneOf branch: {}",
                        errs.iter()
                            .map(|e| e.join("; "))
                            .collect::<Vec<_>>()
                            .join(" | ")
                    )),
                    n => out.push(format!("{at}: matches {n} oneOf branches")),
                }
            }
            "const" | "enum" | "minimum" => {}
            k if PASSIVE.contains(&k) => {}
            other => panic!("schema keyword `{other}` at {at} is not checked here: add it"),
        }
    }
    if let Some(o) = v.as_object() {
        let props = s.get("properties").and_then(Value::as_object);
        for (k, x) in o {
            let sub = format!("{at}.{k}");
            match (props.and_then(|p| p.get(k)), s.get("additionalProperties")) {
                (Some(ps), _) => check(root, ps, x, &sub, depth + 1, out),
                (None, Some(ap)) => check(root, ap, x, &sub, depth + 1, out),
                (None, None) => {}
            }
        }
    }
    // `items` checks the elements past `prefixItems`.
    if let Some(a) = v.as_array() {
        let prefix = s
            .get("prefixItems")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for (i, x) in a.iter().enumerate() {
            let sub = format!("{at}[{i}]");
            match (prefix.get(i), s.get("items")) {
                (Some(p), _) => check(root, p, x, &sub, depth + 1, out),
                (None, Some(it)) => check(root, it, x, &sub, depth + 1, out),
                (None, None) => {}
            }
        }
    }
}

fn errors_under(root: &Value, s: &Value, v: &Value, at: &str, depth: u32) -> Vec<String> {
    let mut out = Vec::new();
    check(root, s, v, at, depth + 1, &mut out);
    out
}

fn is_type(t: &str, v: &Value) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        other => panic!("unknown type name {other}"),
    }
}

mod tests {
    use super::*;
    use serde_json::json;

    /// The checker must fail what the spec says fails: a negative per keyword.
    #[test]
    fn each_keyword_rejects_its_counterexample() {
        let root = json!({
            "$defs": { "N": { "type": "number", "minimum": 0 } },
            "type": "object", "required": ["a"], "additionalProperties": false,
            "properties": {
                "a": { "$ref": "#/$defs/N" },
                "b": { "oneOf": [{ "type": "string" }, { "type": "integer" }] },
                "c": { "type": "array", "prefixItems": [{ "const": 1 }], "items": false, "minItems": 1 },
                "d": { "enum": ["x", "y"] },
            },
        });
        assert_eq!(
            errors(&root, &json!({ "a": 1, "b": "s", "c": [1], "d": "x" })),
            Vec::<String>::new()
        );
        for bad in [
            json!({}),
            json!({ "a": -1 }),
            json!({ "a": "1" }),
            json!({ "a": 1, "z": 0 }),
            json!({ "a": 1, "b": 1.5 }),
            json!({ "a": 1, "c": [2] }),
            json!({ "a": 1, "c": [1, 1] }),
            json!({ "a": 1, "c": [] }),
            json!({ "a": 1, "d": "z" }),
        ] {
            assert!(!errors(&root, &bad).is_empty(), "{bad} passed");
        }
    }
}
