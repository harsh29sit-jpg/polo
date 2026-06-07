use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{error::Error, fact::{Attr, Value}};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttrType {
    Str,
    Int,
    Float,
    Bool,
    Json,
    Any,
}

impl AttrType {
    fn matches(&self, v: &Value) -> bool {
        match self {
            AttrType::Any => true,
            AttrType::Str => matches!(v, Value::Str(_)),
            AttrType::Int => matches!(v, Value::Int(_)),
            AttrType::Float => matches!(v, Value::Float(_)),
            AttrType::Bool => matches!(v, Value::Bool(_)),
            AttrType::Json => matches!(v, Value::Json(_)),
        }
    }
}

impl std::fmt::Display for AttrType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AttrType::Str => "str",
            AttrType::Int => "int",
            AttrType::Float => "float",
            AttrType::Bool => "bool",
            AttrType::Json => "json",
            AttrType::Any => "any",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttrSpec {
    pub attr_type: AttrType,
    pub required: bool,
    pub description: Option<String>,
}

impl AttrSpec {
    pub fn new(t: AttrType) -> Self {
        Self {
            attr_type: t,
            required: false,
            description: None,
        }
    }

    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Schema {
    pub attrs: HashMap<String, AttrSpec>,
    /// Whether to reject attrs not listed in the schema.
    pub strict: bool,
}

impl Schema {
    pub fn validate(&self, attr: &Attr, value: &Value) -> Result<(), Error> {
        match self.attrs.get(attr.as_str()) {
            Some(spec) => {
                if !spec.attr_type.matches(value) {
                    return Err(Error::SchemaViolation {
                        attr: attr.to_string(),
                        reason: format!(
                            "expected {}, got {}",
                            spec.attr_type,
                            value.type_name()
                        ),
                    });
                }
            }
            None if self.strict => {
                return Err(Error::SchemaViolation {
                    attr: attr.to_string(),
                    reason: "attribute not in schema (strict mode)".into(),
                });
            }
            None => {}
        }
        Ok(())
    }
}
