use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct Schema {
    pub components: Components,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Components {
    pub requests: HashMap<String, json_schema::JSONSchemaObject>,
    pub events: HashMap<String, json_schema::JSONSchemaObject>,
}
