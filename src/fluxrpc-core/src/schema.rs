use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Schema {
    pub components: Components,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Components {
    pub requests: HashMap<String, json_schema::JSONSchemaObject>,
    pub events: HashMap<String, json_schema::JSONSchemaObject>,
}
