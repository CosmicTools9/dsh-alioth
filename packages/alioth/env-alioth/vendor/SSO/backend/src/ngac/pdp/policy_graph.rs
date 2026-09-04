use dashmap::DashMap;

use super::*;

#[derive(Debug, Clone)]
pub struct PolicyGraph {
    pub(super) associations: DashMap<i64, NgacAssociation>,
    pub(super) access_rights: DashMap<i64, NgacAccessRight>,
    pub(super) prohibitions: DashMap<i64, NgacProhibition>,
    pub(super) user_attr_index: DashMap<i64, Vec<i64>>,
    pub(super) object_attr_index: DashMap<i64, Vec<i64>>,
    pub(super) access_right_name_index: DashMap<String, i64>,
}

impl PolicyGraph {
    pub fn new() -> Self {
        Self {
            associations: DashMap::new(),
            access_rights: DashMap::new(),
            prohibitions: DashMap::new(),
            user_attr_index: DashMap::new(),
            object_attr_index: DashMap::new(),
            access_right_name_index: DashMap::new(),
        }
    }

    pub fn has_associations(&self) -> bool {
        !self.associations.is_empty()
    }

    pub fn add_association(&self, assoc: NgacAssociation) {
        let fk_ua = assoc.fk_user_attribute;
        let fk_oa = assoc.fk_object_attribute;

        self.associations.insert(assoc.id, assoc.clone());

        self.user_attr_index
            .entry(fk_ua)
            .or_default()
            .push(assoc.id);

        self.object_attr_index
            .entry(fk_oa)
            .or_default()
            .push(assoc.id);
    }

    pub fn add_access_right(&self, ar: NgacAccessRight) {
        self.access_right_name_index
            .insert(ar.o_name.clone(), ar.id);
        self.access_rights.insert(ar.id, ar);
    }

    pub fn add_prohibition(&self, proh: NgacProhibition) {
        self.prohibitions.insert(proh.id, proh);
    }

    /// 移除一条 association（含双侧索引）。仅供影响预览在**克隆图**上使用
    /// （change `add-ngac-audit-trail-view` D2）——运行时图只经 reload 原子换图。
    pub fn remove_association(&self, id: i64) {
        if let Some((_, assoc)) = self.associations.remove(&id) {
            if let Some(mut ids) = self.user_attr_index.get_mut(&assoc.fk_user_attribute) {
                ids.retain(|x| *x != id);
            }
            if let Some(mut ids) = self.object_attr_index.get_mut(&assoc.fk_object_attribute) {
                ids.retain(|x| *x != id);
            }
        }
    }

    /// 移除一条 prohibition（同上，仅预览克隆图用）。
    pub fn remove_prohibition(&self, id: i64) {
        self.prohibitions.remove(&id);
    }
}

impl Default for PolicyGraph {
    fn default() -> Self {
        Self::new()
    }
}
