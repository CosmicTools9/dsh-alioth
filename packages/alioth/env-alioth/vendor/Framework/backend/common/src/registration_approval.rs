//! 注册/入驻审批**通道映射单源**（wire-registration-approval-closure D3）。
//!
//! 「通道」= 审批事件 code →（流程模板 code / 事件叶表 / notice、comments 文案模板 /
//! 账号激活语义）五要素。消费方 MUST 从本表取要素，MUST NOT 各自复制字面值：
//! - SSO `auth/register.rs` 写入端——注册核心按通道取事件 code、流程 code 与文案模板；
//! - Gateway `seed/approval_seed.rs` 启动自检——断链 `oper→even` 重建写哪张叶表、
//!   模板绑定回填目标流程、notice 前缀/后缀提取；
//! - Gateway `api/approvals.rs` 注册审批终态——按 [`activation_event_codes`] 判定哪些
//!   code 享有账号激活/禁用语义（MUST NOT 复制字面 `code IN (…)` 列表与映射表脱钩）。
//!
//! 三通道（内部注册 / 外部主体入驻 / 实名审核）同表遍历 ⇒ 自愈覆盖面同等待遇，
//! MUST NOT 以缺省分支（`_ => None` 等）静默跳过任一已注册通道。
//!
//! 叶表归模型 DDL 所有（`Alioth/002_isahl_tables.sql`）并由模型级种子播种
//! （`Framework/seed/seed-auth-approval-flows.sql`）——本表只声明「写哪张」，
//! 叶表缺失时写径 MUST 显式失败，MUST NOT 降级写域父表（§8.5 叶表规范）。

use sqlx::PgConnection;

/// 审批通道规格：一个事件 code 决定流程模板、事件叶表、文案模板与账号激活语义
/// （五要素单源）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovalChannelSpec {
    /// 审批事件与实例 code（叶表 `code` / `zc_id_oper-approve.code`）。
    pub event_code: &'static str,
    /// 绑定审批流程模板 code（`zc_id_process.code`）。
    pub flow_code: &'static str,
    /// 事件叶表（断链 `oper→even` 重建的写入目标；归模型 DDL 所有）。
    pub leaf_table: &'static str,
    /// 事件 notice 文案模板（`{}` 为申请人标识占位：注册通道 = username，
    /// 实名审核通道 = 用户 id——与各自写入端逐字一致）。
    pub notice_tpl: &'static str,
    /// 事件 comments 文案模板（`{}` 为申请人标识占位；纯文本语义——
    /// remove-comments-json-embedding 规约）。
    pub comments_tpl: &'static str,
    /// 本通道审批**终态**是否授予账号激活/禁用语义（`isahl_auth.auth_users.status`
    /// 迁移 + 实名主体回填）。注册/入驻类通道（内部注册、外部主体入驻）= `true`；
    /// 实名审核（`user-verify`）= `false`——实名审核只落审核结果，MUST NOT 激活账号
    /// （实名不构成访问授权）。判定消费方 MUST 经 [`activation_event_codes`] 取集合，
    /// MUST NOT 复制 `code IN (…)` 字面列表。
    pub grants_account_activation: bool,
}

/// 内部注册通道（Gateway 注册页 `/auth/register`；`approvals/apply` 重新申请复用
/// 同一事件 code）：访问授权审批，FLOW-AUTHORIZATION。
pub const INTERNAL_REGISTRATION_CHANNEL: ApprovalChannelSpec = ApprovalChannelSpec {
    event_code: "user-register-approval",
    flow_code: "FLOW-AUTHORIZATION",
    leaf_table: "zc_id_appr-authorization",
    notice_tpl: "用户 {} 访问授权审批",
    comments_tpl: "访问授权审批：申请人 {}",
    grants_account_activation: true,
};

/// 外部主体入驻通道（OpenActivity 门户 `/auth/register/external`）：独立流程
/// FLOW-EXTERNAL-SUBJECT 与事件 code，事件叶表与内部通道同表。
pub const EXTERNAL_SUBJECT_CHANNEL: ApprovalChannelSpec = ApprovalChannelSpec {
    event_code: "external-subject-register-approval",
    flow_code: "FLOW-EXTERNAL-SUBJECT",
    leaf_table: "zc_id_appr-authorization",
    notice_tpl: "外部主体 {} 入驻审批",
    comments_tpl: "外部主体入驻审批：申请人 {}",
    grants_account_activation: true,
};

/// 实名审核通道（SSO `identity.rs` 写入）：独立叶表（含 `fk_process` 列）与流程
/// FLOW-USER-VERIFY。comments 写侧现值为实名实体结构化载荷（`identity.rs`，
/// 非 `{}` 模板）——本字段给出该通道人读摘要形态（自愈重建消费）。
pub const USER_VERIFY_CHANNEL: ApprovalChannelSpec = ApprovalChannelSpec {
    event_code: "user-verify",
    flow_code: "FLOW-USER-VERIFY",
    leaf_table: "zc_id_appr-user_verify",
    notice_tpl: "用户 {} 实名审核",
    comments_tpl: "实名审核：申请人 {}",
    grants_account_activation: false,
};

/// 全部已注册通道（自检遍历 / 映射消费的唯一清单；新增通道只在此登记）。
pub const APPROVAL_CHANNELS: &[ApprovalChannelSpec] = &[
    INTERNAL_REGISTRATION_CHANNEL,
    EXTERNAL_SUBJECT_CHANNEL,
    USER_VERIFY_CHANNEL,
];

/// 注册类（账号激活语义）通道的**事件 code 集合**——由映射表
/// [`ApprovalChannelSpec::grants_account_activation`] 单源派生。
///
/// 消费方（Gateway `api/approvals.rs` 注册审批终态实现）MUST 经本入口取判定集合
/// （SQL `code = ANY($n::text[])` 绑定），MUST NOT 在 SQL/代码里复制
/// `code IN ('…')` 字面列表——字面列表与映射表脱钩即「新增注册类通道漏激活/漏禁用」
/// 类缺陷（wire-registration-approval-closure C4 第 2 步）。
pub fn activation_event_codes() -> Vec<&'static str> {
    APPROVAL_CHANNELS
        .iter()
        .filter(|ch| ch.grants_account_activation)
        .map(|ch| ch.event_code)
        .collect()
}

/// 注册/入驻审批受理结果（[`intake`] 返回）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IntakeResult {
    /// 审批事件行 id（落在 `channel.leaf_table`）。
    pub event_id: i64,
    /// 审批实例 id（`zc_id_oper-approve`；= SSO 响应体 `approval_instance_id` 载荷）。
    pub instance_id: i64,
}

/// 事件叶表 INSERT 语句（表名静态字面量）。
///
/// 叶表集合编译期闭集 ⇒ 编译期静态分发，MUST NOT 运行期 `format!` 拼表名
/// （check-dynamic-table-name「表名 MUST 编译期静态分发」）。模板/SLA 绑定列可空：
/// 缺失 ⇒ NULL 降级（Gateway seed 组件启动自愈 + 一致性回填）。
const EVENT_INSERT_AUTHORIZATION_LEAF: &str = r#"
INSERT INTO isahl."zc_id_appr-authorization" (
    created_by_id, updated_by_id, notice, code, comments,
    tpl_id, qk_sla, created_at, updated_at,
    dk_scene, dk_factor, dk_function
) VALUES ($1, $1, $2, $3, $4, $5, $6, NOW(), NOW(), $7, $8, $9)
RETURNING id
"#;

/// 叶表 → 事件 INSERT 语句（闭集静态分发；未登记叶表 ⇒ `None`）。
///
/// 内部注册与外部主体入驻同叶表（映射表声明）⇒ 单条语句服务双通道；
/// 实名审核通道写入形态不同（结构化 comments 载荷 + `fk_process`，见 SSO `identity.rs`），
/// 不经本函数。
fn event_insert_sql(leaf_table: &str) -> Option<&'static str> {
    if leaf_table == INTERNAL_REGISTRATION_CHANNEL.leaf_table {
        Some(EVENT_INSERT_AUTHORIZATION_LEAF)
    } else {
        None
    }
}

/// 注册审批**受理**：审批事件行 + `register-context` 上下文操作行 + 流程归属桥 +
/// 模板桥 + `zc_id_oper-approve` 实例（单一实现，wire-registration-approval-closure D2/D4）。
///
/// 消费方：SSO 注册端点（内部/外部双通道）与 Gateway 自助重新申请端点
/// （`/api/approvals/apply`）。语义逐字保持自 SSO `auth/register.rs` 原受理序列：
///
/// - **调用方事务内**执行（`conn` = 调用方 `&mut PgConnection`）；本函数 MUST NOT 另开事务；
/// - 事件写 `channel.leaf_table`（叶表归模型 DDL 所有；未登记叶表 ⇒ 显式失败，
///   MUST NOT 降级写域父表——§8.5 叶表规范）；
/// - 实例 `fk_subject` = 申请人；`fk_operator` = 首个 admin（判据唯一实现 = DB 函数
///   `isahl_auth.is_admin`），bootstrap 阶段无 admin ⇒ NULL，由管理面后补；
/// - 坐标（§6.12 声明即必须）：事件叶表 `JC/FTA/↑_NA`；`register-context` 与实例
///   `JE/FTA/↓_EZ`（值经 `ontology_binding::resolve_conn` 事务内解析，禁硬编码 ZUID）；
/// - 文案：`channel.notice_tpl` / `channel.comments_tpl`，`{}` → `username`；
///   comments 纯文本（comments-text-semantics：写侧 MUST NOT JSON）；
/// - 归属桥链形状（`fk_process` 列已物理移除，remove-event-fk-process-pollution D3 写侧）：
///   `register-context` oper 行（每流程复用一行，模板桥回填目标）→ 归属桥
///   `zc_id_process_rr_operation` → 模板桥 `zc_id_operation_rr_event`（register-context → 事件）
///   → 实例↔事件同表第二桥；id 生成方式与既有逐字一致
///   （`isahl.gen_next_uid(372)` 归属桥 / `isahl.gen_next_uid(267)` 模板桥）；
/// - 模板绑定与 SLA（72h）行缺失 ⇒ NULL 降级不阻断；该降级语义（含查询失败折叠为 NULL）
///   逐字保留自迁移前实现。
pub async fn intake(
    conn: &mut PgConnection,
    channel: &ApprovalChannelSpec,
    user_id: i64,
    username: &str,
) -> Result<IntakeResult, sqlx::Error> {
    let approval_notice = channel.notice_tpl.replace("{}", username);
    let approval_comments = channel.comments_tpl.replace("{}", username);

    // 坐标三元组（§6.12 声明即必须）：`register-context` 与实例两处 `zc_id_oper-approve`
    // INSERT 共用一次解析（循环/多处 → 循环外一次求得）。
    let (dk_scene, dk_factor, dk_function) =
        ontology_binding::resolve_conn(&mut *conn, ("JE", "FTA", "↓_EZ")).await?;

    // 流程模板绑定：flow_code → (流程 id, approve 节点事件模板 id)；缺失降级 NULL 不阻断。
    // 节点类型权威承载 = `zc_id_operation.ck_cate-proc_op`（→ `zc_id_cate-proc_op` 字典），
    // **不在 `code` 列**（code-semantics R1：禁 `code = '<类型字面量>'`）——故以
    // `zc_id_oper-approve` 子类型联表选取 approve 节点（与 seed 自检 flow_binding 同构）。
    let flow_binding: Option<(i64, Option<i64>)> = sqlx::query_as(
        r#"
        SELECT p.id,
               (SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
                JOIN isahl."zc_id_oper-approve" oa
                  ON oa.id = rro.ref_right AND oa.deleted_at IS NULL
                WHERE rro.ref_left = p.id AND rro.deleted_at IS NULL LIMIT 1)
        FROM isahl.zc_id_process p
        WHERE p.code = $1 AND p.deleted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(channel.flow_code)
    .fetch_optional(&mut *conn)
    .await
    .ok()
    .flatten();
    // SLA 时长绑定（add-register-approval-closure 缺口 2）：72h 时长维度行，
    // 缺省 NULL（seed 组件启动预置 + 一致性回填）；超时自动驳回由 SLA 监控处理。
    let sla_duration_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT id FROM isahl."zc_id_scal-duration"
           WHERE o_number = '72h' AND deleted_at IS NULL LIMIT 1"#,
    )
    .fetch_optional(&mut *conn)
    .await
    .ok()
    .flatten();

    // 审批事件（叶表 `channel.leaf_table`，绑 `channel.flow_code` 模板）。
    // 叶表由模型 DDL 拥有并由模型级种子播种 ⇒ 无存在性探测与降级分支；
    // 未登记叶表 = 显式失败（MUST NOT 复制一条 INSERT 伪装降级）。
    let event_id: i64 = {
        // 坐标三元组（§6.12 声明即必须）：审批叶表继承 even-approve 坐标 JC/FTA/↑_NA
        let (leaf_dk_scene, leaf_dk_factor, leaf_dk_function) =
            ontology_binding::resolve_conn(&mut *conn, ("JC", "FTA", "↑_NA")).await?;
        let sql = event_insert_sql(channel.leaf_table).ok_or_else(|| {
            sqlx::Error::Protocol(format!(
                "未登记审批事件叶表（通道映射表声明外）：{}",
                channel.leaf_table
            ))
        })?;
        sqlx::query_scalar(sql)
            .bind(user_id)
            .bind(&approval_notice)
            .bind(channel.event_code)
            .bind(&approval_comments)
            .bind(flow_binding.as_ref().and_then(|(_, tpl)| *tpl))
            .bind(sla_duration_id)
            .bind(leaf_dk_scene)
            .bind(leaf_dk_factor)
            .bind(leaf_dk_function)
            .fetch_one(&mut *conn)
            .await?
    };

    // 事件↔流程归属经桥链：'register-context' 上下文 oper 行（每流程复用一行）
    // + process_rr_operation 归属桥 + rr_event 模板桥。
    if let Some((flow_id, _)) = flow_binding {
        let ctx_oper: Option<i64> = sqlx::query_scalar(
            r#"SELECT rro.ref_right FROM isahl.zc_id_process_rr_operation rro
               JOIN isahl."zc_id_oper-approve" oa ON oa.id = rro.ref_right
                 AND oa.deleted_at IS NULL AND oa.notice = 'register-context'
               WHERE rro.ref_left = $1 AND rro.deleted_at IS NULL LIMIT 1"#,
        )
        .bind(flow_id)
        .fetch_optional(&mut *conn)
        .await?;
        let ctx_oper: i64 = match ctx_oper {
            Some(v) => v,
            None => {
                let new_id: i64 = sqlx::query_scalar(
                    r#"INSERT INTO isahl."zc_id_oper-approve"
                           (notice, created_by_id, dk_scene, dk_factor, dk_function)
                       VALUES ('register-context', $1, $2, $3, $4) RETURNING id"#,
                )
                .bind(user_id)
                .bind(dk_scene)
                .bind(dk_factor)
                .bind(dk_function)
                .fetch_one(&mut *conn)
                .await?;
                sqlx::query(
                    "INSERT INTO isahl.zc_id_process_rr_operation (id, ref_left, ref_right, created_by_id)
                     VALUES (isahl.gen_next_uid(372), $1, $2, $3)",
                )
                .bind(flow_id)
                .bind(new_id)
                .bind(user_id)
                .execute(&mut *conn)
                .await?;
                new_id
            }
        };
        sqlx::query(
            "INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
             VALUES (isahl.gen_next_uid(267), $1, $2, $3)",
        )
        .bind(ctx_oper)
        .bind(event_id)
        .bind(user_id)
        .execute(&mut *conn)
        .await?;
    }

    // 审批实例（zc_id_oper-approve）：fk_operator = 首个 admin（审批工作区按 operator 可见）；
    // 无 admin（bootstrap 阶段）→ NULL，由管理面后补。判据唯一实现 = DB 函数
    // `isahl_auth.is_admin`（有效 UA 集含派生与时效，consolidate-ngac-admin-predicate）。
    let admin_id: Option<i64> = sqlx::query_scalar::<_, i64>(
        "SELECT u.id FROM isahl_auth.auth_users u \
         WHERE isahl_auth.is_admin(u.id) ORDER BY u.id LIMIT 1",
    )
    .fetch_optional(&mut *conn)
    .await?;

    let instance_id: i64 = sqlx::query_scalar(
        r#"
        INSERT INTO isahl."zc_id_oper-approve" (
            notice, code, fk_subject, fk_operator, created_by_id, created_at, updated_at,
            dk_scene, dk_factor, dk_function
        ) VALUES ($1, $2, $3, $4, $3, NOW(), NOW(), $5, $6, $7)
        RETURNING id
        "#,
    )
    .bind(&approval_notice)
    .bind(channel.event_code)
    .bind(user_id)
    .bind(admin_id)
    .bind(dk_scene)
    .bind(dk_factor)
    .bind(dk_function)
    .fetch_one(&mut *conn)
    .await?;

    // 实例↔事件关联（fk_approve 列已物理移除）→ operation_rr_event 桥
    sqlx::query(
        "INSERT INTO isahl.zc_id_operation_rr_event (id, ref_left, ref_right, created_by_id)
         VALUES (isahl.gen_next_uid(267), $1, $2, $3)",
    )
    .bind(instance_id)
    .bind(event_id)
    .bind(user_id)
    .execute(&mut *conn)
    .await?;

    Ok(IntakeResult {
        event_id,
        instance_id,
    })
}

/// 单用户**基础身份 UA**（`user`）幂等指派——唯一实现（`approval-endpoint-gates::user-ua-on-registration`）。
///
/// 消费方 = 注册受理链路（`SSO/backend/src/auth/register.rs`）与 rejected 复用申请链路
/// （`Gateway/backend/src/api/approvals.rs`）；两处 MUST 调用本函数，MUST NOT 各自内联
/// `INSERT INTO isahl_auth.ngac_user_rr_attribute … o_name = 'user'` SQL。
///
/// 语义：① 幂等（`ON CONFLICT (fk_user, fk_user_attribute) DO UPDATE SET deleted_at = NULL`，
/// 复活软删行）；② `user` UA 不存在（未部署 NGAC 面）⇒ 静默跳过，保持既有 best-effort 语义。
/// 函数返回 `Result` 以便未来需 fail-loud 的调用方复用；既有调用点按原语义忽略错误。
pub async fn ensure_user_ua(conn: &mut PgConnection, user_id: i64) -> Result<(), sqlx::Error> {
    let user_ua_id: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute \
         WHERE o_name = 'user' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(ua_id) = user_ua_id {
        sqlx::query(
            "INSERT INTO isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute, o_name) \
             VALUES ($1, $2, 'user') \
             ON CONFLICT (fk_user, fk_user_attribute) \
             DO UPDATE SET deleted_at = NULL, updated_at = NOW()",
        )
        .bind(user_id)
        .bind(ua_id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 通道四要素齐备且与各写入端现值逐字一致（wire-registration-approval-closure 1.1）：
    /// - 内部 / 外部通道 = `SSO/backend/src/auth/register.rs` 现值（写入端直接引用本表）；
    /// - 实名审核 notice = `SSO/backend/src/auth/identity.rs` 现值。
    #[test]
    fn channel_specs_are_complete_and_verbatim() {
        assert_eq!(
            APPROVAL_CHANNELS.len(),
            3,
            "自检遍历面 = 三注册通道（内部注册 / 外部入驻 / 实名审核）"
        );
        for ch in APPROVAL_CHANNELS {
            for (name, v) in [
                ("event_code", ch.event_code),
                ("flow_code", ch.flow_code),
                ("leaf_table", ch.leaf_table),
                ("notice_tpl", ch.notice_tpl),
                ("comments_tpl", ch.comments_tpl),
            ] {
                assert!(!v.is_empty(), "通道 {} 的 {name} MUST 非空", ch.event_code);
            }
            assert_eq!(
                ch.notice_tpl.matches("{}").count(),
                1,
                "通道 {} notice_tpl MUST 含恰好一个占位符",
                ch.event_code
            );
            assert_eq!(
                ch.comments_tpl.matches("{}").count(),
                1,
                "通道 {} comments_tpl MUST 含恰好一个占位符",
                ch.event_code
            );
        }

        let internal = INTERNAL_REGISTRATION_CHANNEL;
        assert_eq!(internal.event_code, "user-register-approval");
        assert_eq!(internal.flow_code, "FLOW-AUTHORIZATION");
        assert_eq!(internal.leaf_table, "zc_id_appr-authorization");
        assert_eq!(internal.notice_tpl, "用户 {} 访问授权审批");
        assert_eq!(internal.comments_tpl, "访问授权审批：申请人 {}");

        let external = EXTERNAL_SUBJECT_CHANNEL;
        assert_eq!(external.event_code, "external-subject-register-approval");
        assert_eq!(external.flow_code, "FLOW-EXTERNAL-SUBJECT");
        assert_eq!(external.leaf_table, "zc_id_appr-authorization");
        assert_eq!(external.notice_tpl, "外部主体 {} 入驻审批");
        assert_eq!(external.comments_tpl, "外部主体入驻审批：申请人 {}");

        let verify = USER_VERIFY_CHANNEL;
        assert_eq!(verify.event_code, "user-verify");
        assert_eq!(verify.flow_code, "FLOW-USER-VERIFY");
        assert_eq!(verify.leaf_table, "zc_id_appr-user_verify");
        assert_eq!(verify.notice_tpl, "用户 {} 实名审核");
        assert_eq!(verify.comments_tpl, "实名审核：申请人 {}");
    }

    /// notice 模板 → 前缀/后缀拆分（自愈侧从 notice 反解申请人名的唯一依据）。
    #[test]
    fn notice_tpl_splits_into_prefix_and_suffix() {
        let (prefix, suffix) = INTERNAL_REGISTRATION_CHANNEL
            .notice_tpl
            .split_once("{}")
            .expect("含占位符");
        assert_eq!(prefix, "用户 ");
        assert_eq!(suffix, " 访问授权审批");
        let (prefix, suffix) = EXTERNAL_SUBJECT_CHANNEL
            .notice_tpl
            .split_once("{}")
            .expect("含占位符");
        assert_eq!(prefix, "外部主体 ");
        assert_eq!(suffix, " 入驻审批");
        let (prefix, suffix) = USER_VERIFY_CHANNEL
            .notice_tpl
            .split_once("{}")
            .expect("含占位符");
        assert_eq!(prefix, "用户 ");
        assert_eq!(suffix, " 实名审核");
    }

    /// 事件叶表 INSERT 闭集分发（wire-registration-approval-closure 2.1）：
    /// 双注册通道同叶表 ⇒ 同一静态语句；未登记叶表 ⇒ `None`（调用方显式失败，不降级）。
    #[test]
    fn event_insert_sql_dispatches_over_registration_leaf() {
        let internal = event_insert_sql(INTERNAL_REGISTRATION_CHANNEL.leaf_table)
            .expect("内部注册通道叶表 MUST 已登记");
        let external = event_insert_sql(EXTERNAL_SUBJECT_CHANNEL.leaf_table)
            .expect("外部主体通道叶表 MUST 已登记");
        assert_eq!(
            internal, external,
            "双注册通道同叶表 ⇒ MUST 复用同一静态语句（单实现）"
        );
        assert!(
            internal.contains(r#"INSERT INTO isahl."zc_id_appr-authorization""#),
            "语句 MUST 静态落在映射表声明的叶表上（编译期静态分发，无运行期拼表名）"
        );
        for col in ["dk_scene", "dk_factor", "dk_function"] {
            assert!(
                internal.contains(col),
                "叶表 INSERT MUST 声明坐标列 {col}（§6.12 声明即必须）"
            );
        }
        assert_eq!(
            event_insert_sql(USER_VERIFY_CHANNEL.leaf_table),
            None,
            "实名审核通道写入形态不同（结构化载荷 + fk_process），MUST NOT 经由受理函数"
        );
        assert_eq!(event_insert_sql("zc_id_not-registered"), None);
    }

    /// 账号激活维度单源（wire-registration-approval-closure C4 第 2 步）：
    /// 通道字段 = 唯一真相源，[`activation_event_codes`] 只是它的投影——
    /// 双向断言保证「新增通道自动进入判定面」且消费方无需复制字面列表。
    #[test]
    fn activation_event_codes_derive_from_channel_specs() {
        const {
            assert!(
                INTERNAL_REGISTRATION_CHANNEL.grants_account_activation,
                "内部注册通道终态 = 账号激活"
            );
            assert!(
                EXTERNAL_SUBJECT_CHANNEL.grants_account_activation,
                "外部主体入驻通道终态 = 账号激活"
            );
            assert!(
                !USER_VERIFY_CHANNEL.grants_account_activation,
                "实名审核通道 MUST NOT 获得账号激活语义（实名不构成访问授权）"
            );
        }

        let derived = activation_event_codes();
        assert_eq!(
            derived,
            vec![
                "user-register-approval",
                "external-subject-register-approval"
            ],
            "派生集合 = 注册类通道事件 code（与 Gateway 迁移前字面列表逐字一致）"
        );
        for ch in APPROVAL_CHANNELS {
            assert_eq!(
                derived.contains(&ch.event_code),
                ch.grants_account_activation,
                "通道 {} 的激活语义 MUST 与派生集合一致（双向投影）",
                ch.event_code
            );
        }
        assert!(
            !derived.contains(&USER_VERIFY_CHANNEL.event_code),
            "实名审核 code MUST NOT 进入激活判定集合（否则 user-verify 会获得账号激活语义）"
        );
    }
}
