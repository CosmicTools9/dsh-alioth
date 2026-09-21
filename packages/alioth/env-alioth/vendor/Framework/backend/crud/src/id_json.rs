//! 查询期 JSON 行的 id 语义值字符串化（`ID_JSON_PRECISION.md`）
//!
//! `SELECT to_jsonb(t)` 会把 `bigint` 列以 JSON **number** 直出。Alioth 模型 id 量级
//! 10^14–10^18，超 `2^53`（`Number.MAX_SAFE_INTEGER` = 9007199254740992）的 id 在前端
//! `JSON.parse` 会被**静默精度截断**（值失真但不报错）：相邻 id 折叠为同一值 ⇒ 下拉
//! 所有选项同值（选中任一都写同一个、回显首项、已选值无法修改）、提交写错外键。
//!
//! `common::serde_zuid` 只覆盖**有 DTO** 的路径；本模块覆盖**动态 JSON 行**路径
//! （ontology leaf/reference 等按表名拼 SQL、无 DTO 的读取）。判据与规约一致：
//! 键名 `id`/`*_id`/`ids`/`*_ids`、物理列前缀 `fk_`/`qk_`/`sk_`/`dk_`/`ak_`/`tpl_`、
//! 桥表列 `ref_left`/`ref_right`，以及前端 camelCase 形态（`*Id`/`*Ids`）。
//!
//! 事故链：2026-08-30 车型字典 → 2026-09-16 适航证件「健康度」（`zc_id_leve-health`
//! 六项 id 98234766872019999..004 全部折叠为 98234766872020000）。

use serde_json::Value;

/// 物理列 id 前缀（与 `ID_JSON_PRECISION.md` 判据一致）
const ID_PREFIXES: [&str; 6] = ["fk_", "qk_", "sk_", "dk_", "ak_", "tpl_"];

/// 键名是否为 id 语义（snake_case 物理列名 + camelCase 前端形态）
pub fn is_id_key(key: &str) -> bool {
    if key == "id" || key == "ids" {
        return true;
    }
    if matches!(key, "ref_left" | "ref_right" | "refLeft" | "refRight") {
        return true;
    }
    if key.ends_with("_id") || key.ends_with("_ids") {
        return true;
    }
    // camelCase：末段为 Id/Ids（大写 I 起，避免 `valid`/`solid` 等词误判）
    if key.ends_with("Id") || key.ends_with("Ids") {
        return true;
    }
    ID_PREFIXES.iter().any(|p| key.starts_with(p))
}

/// 就地把 JSON 对象内 id 语义的整数改写为字符串（覆盖数组元素与嵌套对象）。
///
/// 仅改写**整数**：浮点（金额/比率等）与非数字值原样保留。
pub fn stringify_ids_in_place(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                let id_key = is_id_key(key);
                rewrite(child, id_key);
            }
        }
        Value::Array(items) => {
            for item in items.iter_mut() {
                stringify_ids_in_place(item);
            }
        }
        _ => {}
    }
}

/// 批量：就地字符串化每行的 id 语义值（`list*` 读出路径用；入参为 sqlx 单列元组行）
pub fn stringify_rows(rows: Vec<(Value,)>) -> Vec<Value> {
    rows.into_iter()
        .map(|(mut row,)| {
            stringify_ids_in_place(&mut row);
            row
        })
        .collect()
}

/// 单行：就地字符串化 id 语义值（`get*` 读出路径用；入参为 sqlx 单列元组行）
pub fn stringify_row(row: Option<(Value,)>) -> Option<Value> {
    row.map(|(mut row,)| {
        stringify_ids_in_place(&mut row);
        row
    })
}

fn rewrite(value: &mut Value, id_key: bool) {
    match value {
        Value::Number(_) if id_key => stringify_number(value),
        Value::Array(items) if id_key => {
            for item in items.iter_mut() {
                if item.is_number() {
                    stringify_number(item);
                }
            }
        }
        Value::Object(_) => stringify_ids_in_place(value),
        Value::Array(items) => {
            for item in items.iter_mut() {
                stringify_ids_in_place(item);
            }
        }
        _ => {}
    }
}

fn stringify_number(value: &mut Value) {
    if let Some(i) = value.as_i64() {
        *value = Value::String(i.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn id_keys_recognized() {
        for key in [
            "id",
            "ids",
            "created_by_id",
            "fk_subject",
            "qk_date",
            "sk_unit",
            "dk_scene",
            "ak_permit_user",
            "tpl_id",
            "ref_left",
            "refRight",
            "certTypeId",
            "typeIds",
        ] {
            assert!(is_id_key(key), "{key} 应判为 id 语义");
        }
        for key in [
            "notice", "code", "amount", "valid", "solid", "progress", "lv_value",
        ] {
            assert!(!is_id_key(key), "{key} 不应判为 id 语义");
        }
    }

    #[test]
    fn bigint_ids_become_strings_including_arrays_and_nesting() {
        let mut row = json!({
            "id": 98234766872020004i64,
            "notice": "已符合",
            "fk_subject": 338044226429804i64,
            "ak_permit_user": [1, 338044226429804i64],
            "amount": 12.5,
            "count": 3,
            "nested": { "ref_left": 98234766872019999i64, "label": "x" }
        });
        stringify_ids_in_place(&mut row);

        assert_eq!(row["id"], json!("98234766872020004"));
        assert_eq!(row["fk_subject"], json!("338044226429804"));
        assert_eq!(row["ak_permit_user"], json!(["1", "338044226429804"]));
        assert_eq!(row["nested"]["ref_left"], json!("98234766872019999"));
        // 非 id 语义 / 浮点保持数字
        assert_eq!(row["notice"], json!("已符合"));
        assert_eq!(row["amount"], json!(12.5));
        assert_eq!(row["count"], json!(3));
    }

    #[test]
    fn precision_loss_cluster_is_separated() {
        // 六个相邻 id 在 JS 中会折叠为同一 double；字符串化后互不相同
        let ids = [
            98234766872019999i64,
            98234766872020000i64,
            98234766872020001i64,
            98234766872020002i64,
            98234766872020003i64,
            98234766872020004i64,
        ];
        let mut rows: Vec<Value> = ids.iter().map(|id| json!({ "id": id })).collect(); // id-json-ok 测试夹具：故意构造裸整数以证明折叠
        for r in rows.iter_mut() {
            stringify_ids_in_place(r);
        }
        let distinct = rows
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(distinct.len(), 6);
    }
}
