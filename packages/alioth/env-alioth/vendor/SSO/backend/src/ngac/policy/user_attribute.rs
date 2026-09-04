use serde::{Deserialize, Serialize};

/// User attribute node aligned with ngac_user_attribute schema
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserAttributeNode {
    #[serde(with = "common::serde_zuid")]
    pub id: i64,
    pub o_name: String,
    #[serde(with = "common::serde_zuid::opt", default)]
    pub fk_policy_class: Option<i64>,
    #[serde(with = "common::serde_zuid::seq")]
    pub ancestor_ids: Vec<i64>,
    #[serde(with = "common::serde_zuid::seq")]
    pub children_ids: Vec<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserAttributeGraph {
    pub nodes: Vec<UserAttributeNode>,
}

impl UserAttributeGraph {
    pub fn new() -> Self {
        Self { nodes: vec![] }
    }

    pub fn add_node(&mut self, node: UserAttributeNode) {
        self.nodes.push(node);
    }

    pub fn get_ancestors(&self, node_id: i64) -> Vec<i64> {
        let mut ancestors = vec![];
        self.collect_ancestors(node_id, &mut ancestors);
        ancestors
    }

    fn collect_ancestors(&self, node_id: i64, ancestors: &mut Vec<i64>) {
        for node in &self.nodes {
            if node.id == node_id {
                for &parent_id in &node.ancestor_ids {
                    if !ancestors.contains(&parent_id) {
                        ancestors.push(parent_id);
                        self.collect_ancestors(parent_id, ancestors);
                    }
                }
            }
        }
    }
}

impl Default for UserAttributeGraph {
    fn default() -> Self {
        Self::new()
    }
}
