//! Framework/approval 集成测试共享辅助
use sqlx::PgPool;

/// 生成唯一测试标识
#[allow(dead_code)] // 共享测试辅助：部分测试文件未使用
pub fn test_code(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{}-{}-{}",
        prefix,
        std::process::id(),
        nanos % 1_000_000_000
    )
}

/// 测试 schema 守门
pub async fn setup_test_schema(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
    let db_name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(pool)
        .await?;
    if !db_name.contains("_test") {
        return Err(format!(
            "REFUSED: running integration test on non-test database '{}'",
            db_name,
        )
        .into());
    }
    sqlx::query("CREATE SCHEMA IF NOT EXISTS isahl")
        .execute(pool)
        .await?;
    Ok(())
}

/// 为测试用户授予指定资源的 action 权限（NGAC association）。
///
/// 测试环境的 `ngac_association` 由运行时 admin 配置产生，测试直接插入授权三元组，
/// 使 `require_resource_access` 的严格评估通过。幂等：先删同名旧行再插。
#[allow(dead_code)] // 共享测试辅助：部分测试 crate 未使用
pub async fn grant_user_access(
    pool: &PgPool,
    user_id: i64,
    resource_type: &str,
    actions: &[&str],
) -> Result<(), sqlx::Error> {
    // -1. NGAC 基础数据幂等预置（重置后的测试库无 policy_class/attribute/right，
    //     运行时 admin 初始化不覆盖测试库；此处自愈保证 require_resource_access 可评估）
    let default_class: i64 = match sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    {
        Some(id) => id,
        None => {
            sqlx::query_scalar(
                r#"INSERT INTO isahl_auth.ngac_policy_class (o_name, description)
               VALUES ('default', '测试默认策略类') RETURNING id"#,
            )
            .fetch_one(pool)
            .await?
        }
    };
    match sqlx::query_scalar::<_, i64>(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    {
        Some(_) => {}
        None => {
            sqlx::query(
                r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class)
                   VALUES ('admin', $1)"#,
            )
            .bind(default_class)
            .execute(pool)
            .await?;
        }
    }
    // access_right 按 action 逐个补齐（整批 NOT EXISTS 会因任一已存在而跳过缺失项）
    let missing_rights: Vec<String> = actions.iter().map(|s| s.to_string()).collect();
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_access_right (o_name, description)
           SELECT a.action, 'test seed'
           FROM unnest($1::text[]) AS a(action)
           WHERE NOT EXISTS (
               SELECT 1 FROM isahl_auth.ngac_access_right r WHERE r.o_name = a.action
           )"#,
    )
    .bind(&missing_rights)
    .execute(pool)
    .await?;

    // 0. 确保测试用户存在于 auth_users（NGAC fk_user 外键约束）
    //    SYSTEM_USER_ID 必须以 system 类型建立（ensure_system_user 幂等），
    //    否则会被覆盖为 standard 测试用户，破坏 system_user_test 的断言。
    if user_id == common::SYSTEM_USER_ID {
        common::system_user::ensure_system_user(pool).await?;
    }
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, $2, $2, $3, 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(user_id)
    .bind(format!("test-user-{}", user_id))
    .bind(format!("test-user-{}@test.local", user_id))
    .execute(pool)
    .await?;

    // 1. 用户 → admin user_attribute 映射
    let admin_attr: i64 = sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = 'admin' AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_one(pool)
    .await?;
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute
           (id, o_name, fk_user, fk_user_attribute, assigned_at, created_at)
           VALUES (isahl.gen_next_zuid(), $1, $2, $3, NOW(), NOW())
           ON CONFLICT DO NOTHING"#,
    )
    .bind(format!("grant-{}-{}", resource_type, user_id))
    .bind(user_id)
    .bind(admin_attr)
    .execute(pool)
    .await?;

    // 2. 通配 object_attribute（resource_type='*' 覆盖全部实体；幂等：已存在则复用）
    let obj_attr: i64 = match sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_object_attribute WHERE resource_type = '*' AND fk_resource = 0 AND deleted_at IS NULL LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    {
        Some(id) => id,
        None => sqlx::query_scalar(
            r#"INSERT INTO isahl_auth.ngac_object_attribute
               (id, o_name, fk_policy_class, resource_type, fk_resource, created_at)
               VALUES (isahl.gen_next_zuid(), $1, (SELECT id FROM isahl_auth.ngac_policy_class LIMIT 1), '*', 0, NOW())
               RETURNING id"#,
        )
        .bind(format!("grant-obj-{}-{}", resource_type, user_id))
        .fetch_one(pool)
        .await?,
    };

    // 3. association：admin attr × 通配 object × 指定 actions
    let right_ids: Vec<i64> = sqlx::query_as::<_, (i64,)>(
        "SELECT id FROM isahl_auth.ngac_access_right WHERE o_name = ANY($1)",
    )
    .bind(actions)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(id,)| id)
    .collect();

    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_association
           (id, fk_user_attribute, fk_object_attribute, fk_policy_class, ak_access_rights, created_at)
           VALUES (isahl.gen_next_zuid(), $1, $2, (SELECT id FROM isahl_auth.ngac_policy_class LIMIT 1), $3, NOW())
           ON CONFLICT (fk_user_attribute, fk_object_attribute, fk_policy_class)
           DO UPDATE SET ak_access_rights = ARRAY(SELECT DISTINCT unnest(ngac_association.ak_access_rights || EXCLUDED.ak_access_rights))"#,
    )
    .bind(admin_attr)
    .bind(obj_attr)
    .bind(&right_ids)
    .execute(pool)
    .await?;

    Ok(())
}

/// 消息服务测试替身（fix-approval-action-chain P2-8）：记录投递，零外部副作用。
#[allow(dead_code)] // 共享测试辅助：部分测试文件未使用
pub mod noop_messaging {
    use async_trait::async_trait;
    use common::error::AliothError;
    use common::messaging::{AlertLevel, DeviceCommand, MessageDeliveryInfo, MessagingService};

    #[derive(Debug, Default)]
    pub struct NoopMessaging {
        pub notifications: std::sync::Mutex<Vec<(u64, String, String)>>,
    }

    #[async_trait]
    impl MessagingService for NoopMessaging {
        async fn send_direct(
            &self,
            _from: u64,
            _to: u64,
            _content: &str,
        ) -> Result<(), AliothError> {
            Ok(())
        }
        async fn send_group(
            &self,
            _from: u64,
            _conversation_id: u64,
            _content: &str,
        ) -> Result<(), AliothError> {
            Ok(())
        }
        async fn broadcast(&self, _from: u64, _content: &str) -> Result<(), AliothError> {
            Ok(())
        }
        async fn send_system_notification(
            &self,
            to: u64,
            title: &str,
            content: &str,
        ) -> Result<(), AliothError> {
            self.notifications
                .lock()
                .unwrap()
                .push((to, title.to_string(), content.to_string()));
            Ok(())
        }
        async fn send_alert(
            &self,
            _level: AlertLevel,
            _title: &str,
            _content: &str,
        ) -> Result<(), AliothError> {
            Ok(())
        }
        async fn send_device_command(
            &self,
            _device_id: &str,
            _command: DeviceCommand,
        ) -> Result<(), AliothError> {
            Ok(())
        }
        async fn broadcast_device_command(
            &self,
            _command: DeviceCommand,
        ) -> Result<(), AliothError> {
            Ok(())
        }
        async fn send_raw(
            &self,
            _topic: &str,
            _payload: Vec<u8>,
            _qos: u8,
        ) -> Result<MessageDeliveryInfo, AliothError> {
            Err(AliothError::Internal("noop".into()))
        }
    }
}

/// 确保业务岗位 UA 存在并指派用户为成员（幂等）——P0-1 岗位解析测试目标。
/// 与 grant_user_access 的区别：不给 admin UA / 通配对象授权——被测用户保持
/// 非管理员身份（withdraw 所有权豁免、NGAC 严格评估等场景依赖此差异）。
#[allow(dead_code)]
pub async fn ensure_role_member(
    pool: &PgPool,
    role: &str,
    user_id: i64,
) -> Result<(), sqlx::Error> {
    let default_class: i64 = match sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_policy_class WHERE o_name = 'default' LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    {
        Some(id) => id,
        None => {
            sqlx::query_scalar(
                r#"INSERT INTO isahl_auth.ngac_policy_class (o_name, description)
                   VALUES ('default', '测试默认策略类') RETURNING id"#,
            )
            .fetch_one(pool)
            .await?
        }
    };
    let attr: i64 = match sqlx::query_scalar(
        "SELECT id FROM isahl_auth.ngac_user_attribute WHERE o_name = $1 AND deleted_at IS NULL LIMIT 1",
    )
    .bind(role)
    .fetch_optional(pool)
    .await?
    {
        Some(id) => id,
        None => {
            sqlx::query_scalar(
                r#"INSERT INTO isahl_auth.ngac_user_attribute (o_name, fk_policy_class)
                   VALUES ($1, $2) RETURNING id"#,
            )
            .bind(role)
            .bind(default_class)
            .fetch_one(pool)
            .await?
        }
    };
    // 用户行（无 admin 授权；system 用户保护）
    if user_id == common::SYSTEM_USER_ID {
        common::system_user::ensure_system_user(pool).await?;
    }
    sqlx::query(
        r#"INSERT INTO isahl_auth.auth_users
           (id, name, username, email, user_type, is_active, created_at, updated_at,
            failed_login_attempts, notification_preferences)
           VALUES ($1, $2, $2, $3, 'standard', TRUE, NOW(), NOW(), 0, '{}'::jsonb)
           ON CONFLICT (id) DO NOTHING"#,
    )
    .bind(user_id)
    .bind(format!("role-user-{}", user_id))
    .bind(format!("role-user-{}@test.local", user_id))
    .execute(pool)
    .await?;
    sqlx::query(
        r#"INSERT INTO isahl_auth.ngac_user_rr_attribute
           (id, o_name, fk_user, fk_user_attribute, assigned_at, created_at)
           SELECT isahl.gen_next_zuid(), $1, $2, $3, NOW(), NOW()
           WHERE NOT EXISTS (
               SELECT 1 FROM isahl_auth.ngac_user_rr_attribute
               WHERE fk_user = $2 AND fk_user_attribute = $3 AND deleted_at IS NULL
           )"#,
    )
    .bind(format!("role-{}-{}", role, user_id))
    .bind(user_id)
    .bind(attr)
    .execute(pool)
    .await?;
    Ok(())
}

/// 审批节点模型接线（fix-avic-approval-node-model 契约）：
/// 节点 = 操作叶行（zc_id_oper-approve），审批人 = 操作→岗位桥（rr_approve），
/// 签署模式 = 操作分类（cate-proc_op code）。comments 不承载结构。
///
/// 对齐 AVIC seed 门禁评审接线（seed-avic-caasec-business.sh）：
/// cate-proc_op（唯一 code）→ operation 叶行（rr_event 桥 ref_right=event_id）→
/// rr_event 桥（operation↔event）→ subj-position 行（唯一 code，fk_user）→
/// rr_approve 桥（operation↔position）→ 回填 operation.ck_cate-proc_op。
///
/// 幂等性由调用方保证（测试各自唯一 code/event）；重跑同一事件会重复接线——
/// 测试事件 id 每次 gen_next_zuid，无冲突。
#[allow(dead_code)]
/// 审批节点接线（refactor-flow-node-operation-model：节点=操作）。
/// 入参 op_id = 节点 operation 行（make_flow/add_approve_node 已创建）：
/// 回填 rr_event 桥=模板（经模板桥反查）+ 岗位桥（rr_approve）+ 签署模式。
pub async fn wire_approval_node(
    pool: &PgPool,
    op_id: i64,
    assignees: &[i64],
    sign_mode: &str,
) -> Result<(), sqlx::Error> {
    let tag = test_code("op");
    let op_code = format!("TST-OPER-{tag}");

    // 1. 签署模式分类：code MUST 为 and_sign/or_sign 字面量——SignMode::parse
    //    读 c.code 映射签署模式；notice 唯一（避免跨测试混淆），code 允许重复
    //    （解析按 code 语义匹配，重复行同义）。
    let mode_id: i64 = sqlx::query_scalar(
        r#"INSERT INTO isahl."zc_id_cate-proc_op" (id, notice, code, enable)
           VALUES (isahl.gen_next_uid(360), $1, $2, true)
           RETURNING id"#,
    )
    .bind(format!("测试签署模式-{tag}"))
    .bind(sign_mode)
    .fetch_one(pool)
    .await?;

    // 2. 模板反查（make_flow 新形态已建 operation→even-approve 模板桥）
    let template_id: Option<i64> = sqlx::query_scalar(
        r#"SELECT oe.ref_right FROM isahl.zc_id_operation_rr_event oe
           WHERE oe.ref_left = $1 AND oe.deleted_at IS NULL
           ORDER BY oe.created_at LIMIT 1"#,
    )
    .bind(op_id)
    .fetch_optional(pool)
    .await?
    .flatten();

    // 3. 模板关联经 rr_event 桥（fk_approve 列已移除）——make_flow/add_node 已建桥
    let _ = template_id;

    // 4. 岗位行（唯一 code，fk_user=审批人）+ 操作 ↔ 岗位桥
    for (idx, user) in assignees.iter().enumerate() {
        let pos_code = format!("{op_code}-POS-{idx}");
        let pos_id: i64 = sqlx::query_scalar(
            r#"INSERT INTO isahl."zc_id_subj-position" (id, notice, code, fk_user)
               VALUES (isahl.gen_next_zuid(), $1, $2, $3)
               RETURNING id"#,
        )
        .bind(format!("测试岗位-{idx}"))
        .bind(&pos_code)
        .bind(user)
        .fetch_one(pool)
        .await?;
        sqlx::query(
            r#"INSERT INTO isahl.zc_id_operation_rr_approve (id, ref_left, ref_right)
               VALUES (isahl.gen_next_zuid(), $1, $2)"#,
        )
        .bind(op_id)
        .bind(pos_id)
        .execute(pool)
        .await?;
    }

    // 5. 签署模式回填
    sqlx::query(
        r#"UPDATE isahl."zc_id_oper-approve"
           SET "ck_cate-proc_op" = $1 WHERE id = $2"#,
    )
    .bind(mode_id)
    .bind(op_id)
    .execute(pool)
    .await?;

    Ok(())
}
