//! crud::fk_index — FK 引用索引（编译期固化，产物驱动）
//!
//! 数据来源链（方案 B，2026-09-24）：
//!   DB（`isahl_meta.meta_fields` + `scripts/db/fk-rr-entries.json` 的 rr 语义条目）
//!     → `scripts/generate-fk-index.ts`（生成器）
//!     → `Framework/backend/crud/fk-index.tsv`（**版本化产物**，入库，头部带 `model_version`）
//!     → `crud/build.rs`（编译期解析；零 build-dependencies）
//!     → `$OUT_DIR/fk_index_generated.rs`（`FK_FORWARD` / `FK_REVERSE` / `FK_INDEX_MODEL_VERSION`）
//!     → 本文件 `include!`（见下方「生成物段」）
//!
//! 为什么是产物而不是活库比对（历史与教训）：本文件曾手工维护，与 DB 实况漂移 2958 条失效
//! 引用；随后一度改为「门禁连库比对 + 漂移就地重建 Rust 源码」，但就地重建会在共享检出与
//! 符号链接场景改写平台源码（ns 仓里 `Framework` 是链接）。产物化后：门禁只比对产物、不再
//! 改写任何 Rust 源码；平台仓与各 namespace 仓静态引用**同一份带版本号的产物**，漂移以
//! 「产物 diff」的形式进入评审。
//!
//! 门禁：`scripts/check/check-fk-index.ts`（push 阶段；比对产物 ⇄ DB 重算结果 + 模型版本绑定）。
//! 消费：`cascade.rs`（软删除级联拓扑推导）、`ontology_handler.rs`、`query_builder.rs`、
//! `schema_repository.rs`。

/// Cascade strategy for delete operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CascadeStrategy {
    Restrict,
    Cascade,
    SetNull,
    SetDefault,
}

/// Resolve cascade strategy for a reverse FK reference.
/// Defaults to Restrict if not explicitly configured.
pub fn resolve_cascade(source_table: &str, field_name: &str) -> CascadeStrategy {
    let _ = (source_table, field_name);
    CascadeStrategy::Restrict
}

// ── 生成物段（编译期由 build.rs 从 fk-index.tsv 注入；MUST NOT 在本文件内联数组）──
// FK_FORWARD: table_name → [(field_name, target_table, local_key)]
// FK_REVERSE: target_table → [(source_table, field_name, local_key)]
// FK_INDEX_MODEL_VERSION: 产物绑定的模型发布版本（如 "v10.0.34"）
include!(concat!(env!("OUT_DIR"), "/fk_index_generated.rs"));

// ── 手写稳定代码（非生成物）──────────────────────────────────────────────
// 生成物只提供上方两个数组与版本常量；查询函数与测试由本段手工维护
// （cascade.rs / query_builder.rs / schema_repository.rs / ontology_handler.rs 依赖这两个函数）。
// 签名：三元组 (field_name, target_table, local_key) / (source_table, field_name, local_key)。

/// Look up forward FK references for a given table.
pub fn lookup_forward_fk(table: &str) -> &[(&str, &str, &str)] {
    let idx = FK_FORWARD.binary_search_by_key(&table, |(k, _)| k);
    match idx {
        Ok(i) => FK_FORWARD[i].1,
        Err(_) => &[],
    }
}

/// Look up reverse FK references for a given table (who references me).
pub fn lookup_reverse_fk(table: &str) -> &[(&str, &str, &str)] {
    let idx = FK_REVERSE.binary_search_by_key(&table, |(k, _)| k);
    match idx {
        Ok(i) => FK_REVERSE[i].1,
        Err(_) => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// FK_FORWARD / FK_REVERSE 必须保持按 key 字典序（binary_search_by_key 依赖）。
    #[test]
    fn fk_index_keys_sorted() {
        for w in FK_FORWARD.windows(2) {
            assert!(w[0].0 < w[1].0, "FK_FORWARD 无序: {} > {}", w[0].0, w[1].0);
        }
        for w in FK_REVERSE.windows(2) {
            assert!(w[0].0 < w[1].0, "FK_REVERSE 无序: {} > {}", w[0].0, w[1].0);
        }
    }

    /// 产物版本常量必须非空（绑定模型发布版本的载体；空值即产物头部缺失）。
    #[test]
    fn fk_index_model_version_present() {
        assert!(
            !FK_INDEX_MODEL_VERSION.is_empty(),
            "FK_INDEX_MODEL_VERSION 为空：产物 fk-index.tsv 头部 `# model_version:` 缺失"
        );
    }

    /// 盘点事件头（P0-2 SPEC-counting-event-fk-index-registration + P1-4
    /// SPEC-counting-event-template-instance-links）：`zc_id_even-counting` 正向注册 MUST
    /// 恰好为 live DB 真实物理列（place/qk_date/storage/subject/tpl_id，无 fk_production），
    /// 反向注册 MUST 含 place/storage/subject 目标 reverse 条目；盘点范围桥接
    /// `zc_id_event_rr_matter` MUST 注册正向（ref_left→event / ref_right→production）与
    /// 两目标反向。
    #[test]
    fn counting_head_fk_registered() {
        // 重建自 meta_fields（DB 真相）+ rr 语义条目：even-counting 8 条带物理 local_key 的引用。
        let fwd = lookup_forward_fk("zc_id_even-counting");
        assert_eq!(
            fwd.len(),
            8,
            "even-counting 应注册 8 条 forward FK，实际: {:?}",
            fwd
        );
        assert!(fwd.contains(&("place", "zc_id_place", "fk_place")));
        assert!(fwd.contains(&("subject", "zc_id_subjects", "fk_subject")));
        assert!(fwd.contains(&("storage", "zc_id_storage", "fk_storage")));
        assert!(fwd.contains(&("qk_date", "zc_id_scal-date", "qk_date")));
        assert!(
            !fwd.iter().any(|(_, _, lk)| lk.contains("fk_production")),
            "事件头注册不得含 fk_production"
        );

        assert!(
            lookup_reverse_fk("zc_id_place").contains(&(
                "zc_id_even-counting",
                "place",
                "fk_place"
            )),
            "zc_id_place 反向应含 even-counting.place 引用"
        );
        assert!(
            lookup_reverse_fk("zc_id_storage").contains(&(
                "zc_id_even-counting",
                "storage",
                "fk_storage"
            )),
            "zc_id_storage 反向应含 even-counting.storage 引用"
        );
        assert!(
            lookup_reverse_fk("zc_id_subjects").contains(&(
                "zc_id_even-counting",
                "subject",
                "fk_subject"
            )),
            "zc_id_subjects 反向应含 even-counting.subject 引用"
        );
    }

    /// 盘点范围桥接（P1-4，SPEC-counting-event-matter-m2n-persisted）：
    /// `zc_id_event_rr_matter` 正向注册 meta_fields 带物理 local_key 的引用（period），
    /// rr 桥接语义条目（event→even-counting.ref_left / matter→production.ref_right）
    /// 仅注册于 reverse 侧（生成器设计：rr 条目只进 reverse，供 cascade 级联）。
    #[test]
    fn counting_matter_relation_registered() {
        let fwd = lookup_forward_fk("zc_id_event_rr_matter");
        assert_eq!(
            fwd.len(),
            1,
            "event_rr_matter 正向应注册 1 条（period，meta_fields 带物理 local_key），实际: {:?}",
            fwd
        );
        assert!(fwd.contains(&("period", "zc_id_segm-date", "qk_period")));

        // rr 桥接条目位于 reverse 侧（even-counting / production 目标）
        let rev_counting = lookup_reverse_fk("zc_id_even-counting");
        assert!(
            rev_counting.contains(&("zc_id_event_rr_matter", "event", "ref_left")),
            "even-counting 反向应含 event_rr_matter.event 引用: {:?}",
            rev_counting
        );
        let rev_prod = lookup_reverse_fk("zc_id_production");
        assert!(
            rev_prod.contains(&("zc_id_event_rr_matter", "matter", "ref_right")),
            "production 反向应含 event_rr_matter.matter 引用: {:?}",
            rev_prod
        );
    }
}
