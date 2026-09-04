//! NGAC 属性图写一致性校验（父存在 / 同策略类 / 自引用 / 环检测）。
//!
//! 所有写 `ancestor_ids` 的路径（admin handlers、LDAP 同步）MUST 复用
//! [`validate_ancestors`]，禁止在调用方复制一份校验逻辑。校验只读不写；写入方应
//! 在**同一事务**内先校验后写入（`admin` handlers 用 `with_validated_write`，
//! LDAP 单条 INSERT 在 `pool.begin()` 事务内调用本函数）。

use sqlx::{Executor, PgConnection, PgPool, Postgres, Transaction};
use std::collections::HashSet;

/// 校验 `ancestors` 作为 `kind` 属性（id=`self_id`）的父属性集合是否合法。
///
/// - 空集合：恒合法（顶层属性）。
/// - 自引用：`self_id` 出现在 `ancestors` → 拒绝。
/// - 父存在：每个父 id 必须存在且未软删。
/// - 同策略类：父子 `fk_policy_class` 均有值且不等 → 拒绝。
/// - 环检测：从候选父 BFS 沿 `ancestor_ids` 展开，触及 `self_id`（更新场景）或
///   重新触及已访问节点（既有脏数据环）→ 拒绝。
///
/// 接受任何 sqlx executor（`&PgPool` / `&mut PgConnection` / `&mut Transaction`），
/// 便于调用方在事务内复用同一实现。
pub async fn validate_ancestors<'e, E>(
    executor: E,
    kind: &str,
    self_id: Option<i64>,
    ancestors: &[i64],
    fk_policy_class: Option<i64>,
) -> Result<(), String>
where
    E: Executor<'e, Database = Postgres>,
{
    if ancestors.is_empty() {
        return Ok(());
    }
    if let Some(sid) = self_id {
        if ancestors.contains(&sid) {
            return Err("不能以自身为父属性".to_string());
        }
    }

    let rows: Vec<(i64, Option<i64>, Vec<i64>)> = match kind {
        "user_attribute" => {
            sqlx::query_as(
                "SELECT id, fk_policy_class, COALESCE(ancestor_ids, '{}') \
                 FROM isahl_auth.ngac_user_attribute WHERE deleted_at IS NULL",
            )
            .fetch_all(executor)
            .await
        }
        "object_attribute" => {
            sqlx::query_as(
                "SELECT id, fk_policy_class, COALESCE(ancestor_ids, '{}') \
                 FROM isahl_auth.ngac_object_attribute WHERE deleted_at IS NULL",
            )
            .fetch_all(executor)
            .await
        }
        _ => return Err(format!("未知属性类型: {}", kind)),
    }
    .map_err(|e| format!("读取属性层级失败: {}", e))?;

    let by_id: std::collections::HashMap<i64, (Option<i64>, Vec<i64>)> = rows
        .into_iter()
        .map(|(id, pc, anc)| (id, (pc, anc)))
        .collect();

    for a in ancestors {
        if !by_id.contains_key(a) {
            return Err(format!("父属性 {} 不存在或已删除", a));
        }
    }

    if let Some(pc) = fk_policy_class {
        for a in ancestors {
            if let Some((Some(apc), _)) = by_id.get(a) {
                if *apc != pc {
                    return Err("父属性与当前属性须同属一个策略类".to_string());
                }
            }
        }
    }

    // 环检测：DFS 沿 ancestor_ids，用「当前路径栈」判定——同一路径上再次出现才为环；
    // 跨分支重复访问共享祖先（菱形 DAG）是合法继承，不得误判。
    // 每个候选父独立 DFS（路径栈清空）；self_id 出现即环（更新场景）。
    fn detect_cycle(
        by_id: &std::collections::HashMap<i64, (Option<i64>, Vec<i64>)>,
        node: i64,
        self_id: Option<i64>,
        path: &mut HashSet<i64>,
    ) -> bool {
        if let Some(sid) = self_id {
            if node == sid {
                return true;
            }
        }
        if !path.insert(node) {
            return true; // 当前路径上再次出现 → 环
        }
        let cycle = by_id
            .get(&node)
            .is_some_and(|(_, anc)| anc.iter().any(|&a| detect_cycle(by_id, a, self_id, path)));
        path.remove(&node);
        cycle
    }

    for a in ancestors {
        let mut path: HashSet<i64> = HashSet::new();
        if detect_cycle(&by_id, *a, self_id, &mut path) {
            return Err("检测到属性继承环".to_string());
        }
    }

    Ok(())
}

/// 事务化「校验 + 写入」：开启事务 → 校验 → 回调写入 → 提交；任一步失败自动回滚。
/// 供 admin handlers 使用（LDAP 单条 INSERT 直接在 `pool.begin()` 事务内调
/// [`validate_ancestors`] 即可）。
pub async fn with_validated_write<T, F>(
    pool: &PgPool,
    kind: &str,
    self_id: Option<i64>,
    ancestors: &[i64],
    fk_policy_class: Option<i64>,
    write: F,
) -> Result<T, String>
where
    F: for<'c> FnOnce(
            &'c mut PgConnection,
        ) -> futures::future::BoxFuture<'c, Result<T, sqlx::Error>>
        + Send,
    T: Send + 'static,
{
    let mut tx: Transaction<'_, Postgres> = pool
        .begin()
        .await
        .map_err(|e| format!("开启事务失败: {}", e))?;
    validate_ancestors(&mut *tx, kind, self_id, ancestors, fk_policy_class).await?;
    let result = write(&mut tx)
        .await
        .map_err(|e| format!("写入失败: {}", e))?;
    tx.commit().await.map_err(|e| format!("提交失败: {}", e))?;
    Ok(result)
}
