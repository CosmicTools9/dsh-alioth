//! NGAC 扩展表 + 运行时结构幂等自愈（零 DDL 交付收敛，add-ngac-runtime-ensure）
//!
//! 项目零 DDL 交付裁决（remove-ddl-delivery-pipeline）：无 DDL 交付物文件，
//! 基础设施表由代码运行时 ensure 幂等自愈（对齐 Gateway seed
//! `ensure_gateway_seed_self_check` 先例）。本模块自愈范围：
//!
//! 1. **扩展表建表**（业务扩展三张 + D-1 规范资产三张）：
//!    - `ngac_access_request`（权限申请，add-ngac-access-request）
//!    - `ngac_delegation`（通用委托，add-ngac-delegation）
//!    - `ngac_binding_request`（绑定申请，add-ngac-binding-request）
//!    - `org_policy_class` / `org_policy_rule` / `org_policy_label`
//!      （规范资产线，d1-org-policy-assets；state 链 CHECK + 审计列；
//!       label 表 code 主键，label_code 仅存 code 字符串不 FK 冻结域）
//! 2. **030 版本信号自愈**：`ngac_policy_version` 兜底 + 030 bump 触发器
//!    （幂等迁移整文件 include_str 执行，同源零复制——无触发器时策略图永不
//!    reload，决策与库内策略脱节）。
//!    - **指派/委托 bump 触发器**（§2.5，fix-ngac-decision-consistency D5）：
//!      `ngac_user_rr_attribute` / `ngac_delegation` 变更 bump 版本——PEP 版本探针
//!      据此在 ≤2s 内失效全 worker 缓存（委托撤销「即时生效」兑现）。
//!      显式不挂 `ngac_user_attribute`（§2.2.3 禁止 UA bump）与
//!      `ngac_object_attribute`（CRUD 建行自动注册 OA，bump 会导致缓存风暴）。
//! 3. **核心约束自愈**：NGAC 核心表主键 + 唯一索引 + 前置去重（防 FK 引用
//!    失败与 ON CONFLICT 无推理目标；针对被无约束重建过的库）。
//! 4. **审计分区兜底**：`ngac_policy_audit_log` 当月/次月/DEFAULT 分区
//!    （032 同源语义；缺分区时审计 INSERT 报「没有为行找到分区」）。
//! 5. **id 默认全量自愈**（§1.5，fix-sso-id-default-heal）：动态扫描
//!    `isahl_auth` 全部含 `id bigint` 且无默认值的关系（含审计分区）幂等补
//!    `gen_next_zuid()`；显式补 `isahl_audit.audit_events`（SSO 自有表，
//!    不批量触碰 isahl_audit 其余 Gateway 管辖表）。
//!
//! **挂载点**（per-process 一次，AtomicBool 免每请求开销）：
//! - `Pdp::ensure_policy_loaded`——PDP 读路径（decide/explain/review/matrix）全覆盖。
//!   关键：`DELEGATED_CTE` 每决策引用 `ngac_delegation`，缺表将导致全部决策
//!   fail-closed 瘫痪，故 PDP 入口 MUST 先行 ensure。
//! - `/auth/me` 权限矩阵（派生名查询同样引用 ngac_delegation）。
//! - access_request / delegation / binding_request 全部 handler 入口。
//!
//! **失败语义**：任一环节失败仅 `log::warn` 不阻断（Gateway seed 自愈同语义）；
//! 失败也置 ensured 位避免每请求重试风暴——缺失结构由决策路径 fail-closed
//! 兜底，下次进程重启重试。
//!
//! 测试侧（`tests/common`）MUST 复用本模块（同源义务，禁止第二份建表 SQL）——
//! `setup_schema` 调用本函数完成测试库同源自愈；auth_users 建表+自愈已收编 §1.5
//! （isahl_auth 基础设施表零 DDL 交付范围）。

use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, Ordering};

static EXTENSION_TABLES_ENSURED: AtomicBool = AtomicBool::new(false);

/// NGAC 扩展表 + 运行时结构幂等自愈（进程级一次）。
pub async fn ensure_ngac_extension_tables(pool: &PgPool) {
    if EXTENSION_TABLES_ENSURED.load(Ordering::Relaxed) {
        return;
    }
    // 1. 扩展表（表 + 索引；编译期字面量，AssertSqlSafe 显式审计；
    //    多语句经 raw_sql 执行——prepared 协议不支持多语句）。
    //    前三条：业务扩展表；后三条：D-1 规范资产表（org_policy_class/
    //    rule/label，d1-org-policy-assets；rule 的 policy_class_id 依赖
    //    class 先建——同循环顺序保证）
    for stmt in [
        // 权限申请（add-ngac-access-request D1）
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.ngac_access_request (
            id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
            fk_user BIGINT NOT NULL REFERENCES isahl_auth.auth_users(id),
            resource_type VARCHAR(64) NOT NULL,
            action VARCHAR(64) NOT NULL,
            reason TEXT,
            status VARCHAR(16) NOT NULL DEFAULT 'pending',
            fk_assignee_ua BIGINT REFERENCES isahl_auth.ngac_user_attribute(id),
            expires_at TIMESTAMPTZ,
            reviewed_by BIGINT REFERENCES isahl_auth.auth_users(id),
            reviewed_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            deleted_at TIMESTAMPTZ
        );
        CREATE INDEX IF NOT EXISTS idx_ngac_access_request_user
            ON isahl_auth.ngac_access_request (fk_user) WHERE deleted_at IS NULL;
        CREATE INDEX IF NOT EXISTS idx_ngac_access_request_status
            ON isahl_auth.ngac_access_request (status, id) WHERE deleted_at IS NULL"#,
        // 通用委托（add-ngac-delegation D1）
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.ngac_delegation (
            id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
            fk_delegator BIGINT NOT NULL REFERENCES isahl_auth.auth_users(id),
            fk_delegatee BIGINT NOT NULL REFERENCES isahl_auth.auth_users(id),
            fk_user_attribute BIGINT NOT NULL REFERENCES isahl_auth.ngac_user_attribute(id),
            date_st TIMESTAMPTZ NOT NULL,
            date_ed TIMESTAMPTZ NOT NULL,
            status VARCHAR(16) NOT NULL DEFAULT 'active',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            deleted_at TIMESTAMPTZ,
            CONSTRAINT ck_ngac_delegation_window CHECK (date_ed > date_st)
        );
        CREATE INDEX IF NOT EXISTS idx_ngac_delegation_delegatee
            ON isahl_auth.ngac_delegation (fk_delegatee, status) WHERE deleted_at IS NULL;
        CREATE INDEX IF NOT EXISTS idx_ngac_delegation_delegator
            ON isahl_auth.ngac_delegation (fk_delegator, status) WHERE deleted_at IS NULL"#,
        // 绑定申请（add-ngac-binding-request D1）
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.ngac_binding_request (
            id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
            fk_user BIGINT NOT NULL REFERENCES isahl_auth.auth_users(id),
            kind VARCHAR(16) NOT NULL,
            target_id BIGINT NOT NULL,
            reason TEXT,
            status VARCHAR(16) NOT NULL DEFAULT 'pending',
            reviewed_by BIGINT REFERENCES isahl_auth.auth_users(id),
            reviewed_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            deleted_at TIMESTAMPTZ,
            CONSTRAINT ck_ngac_binding_request_kind CHECK (kind IN ('entity', 'position'))
        );
        CREATE INDEX IF NOT EXISTS idx_ngac_binding_request_user
            ON isahl_auth.ngac_binding_request (fk_user, status) WHERE deleted_at IS NULL"#,
        // 规范资产（D-1 org-policy-assets；DDL 概念见 ngac-org-phase-d1-spec §2）。
        // class：draft→in_review→active→retired 状态链 + 新版本链；部分唯一
        // (code WHERE deleted_at IS NULL AND state <> 'retired') 语义由服务层
        // 保证，此处落 (code, version) 全唯一 + state 部分索引
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.org_policy_class (
            id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
            code VARCHAR(64) NOT NULL,
            notice TEXT NOT NULL,
            state TEXT NOT NULL DEFAULT 'draft'
                CHECK (state IN ('draft', 'in_review', 'active', 'retired')),
            version INT NOT NULL DEFAULT 1,
            scope JSONB NOT NULL DEFAULT '{}',
            ua_template JSONB NOT NULL DEFAULT '{}',
            label_code VARCHAR(64),
            prohibition_template JSONB,
            audit_required BOOLEAN NOT NULL DEFAULT TRUE,
            effective_from TIMESTAMPTZ,
            effective_until TIMESTAMPTZ,
            comments TEXT,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            created_by_id BIGINT,
            updated_by_id BIGINT,
            deleted_at TIMESTAMPTZ,
            deleted_by_id BIGINT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS uq_org_policy_class_code_version
            ON isahl_auth.org_policy_class (code, version);
        CREATE INDEX IF NOT EXISTS idx_org_policy_class_state
            ON isahl_auth.org_policy_class (state) WHERE deleted_at IS NULL"#,
        // rule：class 挂接的职责规则行（状态链由所属 class 承载；本表 state
        // 默认 active 预留行级生命周期）
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.org_policy_rule (
            id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
            policy_class_id BIGINT NOT NULL
                REFERENCES isahl_auth.org_policy_class (id),
            subject_code VARCHAR(64) NOT NULL,
            resource_type VARCHAR(64) NOT NULL,
            actions JSONB NOT NULL,
            condition JSONB,
            obligation JSONB,
            label_code VARCHAR(64),
            state TEXT NOT NULL DEFAULT 'active',
            version INT NOT NULL DEFAULT 1,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            created_by_id BIGINT,
            updated_by_id BIGINT,
            deleted_at TIMESTAMPTZ,
            deleted_by_id BIGINT
        );
        CREATE INDEX IF NOT EXISTS idx_org_policy_rule_class
            ON isahl_auth.org_policy_rule (policy_class_id, state)
            WHERE deleted_at IS NULL"#,
        // label：分级字典（业务 code 主键；软删 + is_active）
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.org_policy_label (
            code VARCHAR(64) PRIMARY KEY,
            rank INT NOT NULL,
            domain VARCHAR(32) NOT NULL DEFAULT 'security',
            notice TEXT NOT NULL,
            is_active BOOLEAN NOT NULL DEFAULT TRUE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            created_by_id BIGINT,
            updated_by_id BIGINT,
            deleted_at TIMESTAMPTZ,
            deleted_by_id BIGINT
        );
        CREATE INDEX IF NOT EXISTS idx_org_policy_label_active
            ON isahl_auth.org_policy_label (is_active) WHERE deleted_at IS NULL"#,
    ] {
        if let Err(e) = sqlx::raw_sql(sqlx::AssertSqlSafe(stmt)).execute(pool).await {
            log::warn!(
                "ensure_ngac_extension_tables: 扩展表自愈失败（重启后重试）: {}",
                e
            );
            EXTENSION_TABLES_ENSURED.store(true, Ordering::Relaxed);
            return;
        }
    }

    // 1.5 auth_users 幂等自愈（schema 对齐 git 456b0fa51 版 001；isahl_auth
    //    基础设施表由运行时 ensure 自愈——零 DDL 交付裁决范围）
    if let Err(e) = sqlx::raw_sql(sqlx::AssertSqlSafe(
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.auth_users (
            id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
            username TEXT UNIQUE,
            name TEXT,
            email TEXT UNIQUE,
            password_hash TEXT,
            phone TEXT,
            full_name TEXT,
            avatar_url TEXT,
            display_name TEXT,
            is_active BOOLEAN DEFAULT true,
            is_verified BOOLEAN DEFAULT false,
            mfa_enabled BOOLEAN DEFAULT false,
            mfa_secret TEXT,
            mfa_bypass_codes TEXT[],
            is_ldap_user BOOLEAN DEFAULT false,
            ldap_dn TEXT,
            entity_table TEXT,
            entity_id BIGINT,
            settings JSONB DEFAULT '{}',
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );
        DELETE FROM isahl_auth.auth_users a
        USING isahl_auth.auth_users b
        WHERE a.id > b.id AND a.email IS NOT NULL AND a.email = b.email;
        CREATE UNIQUE INDEX IF NOT EXISTS auth_users_email_key
            ON isahl_auth.auth_users (email);
        DELETE FROM isahl_auth.auth_users a
        USING isahl_auth.auth_users b
        WHERE a.id > b.id AND a.username IS NOT NULL AND a.username = b.username;
        CREATE UNIQUE INDEX IF NOT EXISTS auth_users_username_key
            ON isahl_auth.auth_users (username);
        CREATE INDEX IF NOT EXISTS idx_auth_users_username ON isahl_auth.auth_users(username);
        CREATE INDEX IF NOT EXISTS idx_auth_users_email ON isahl_auth.auth_users(email);
        CREATE INDEX IF NOT EXISTS idx_auth_users_active ON isahl_auth.auth_users(is_active);
        -- id 默认全量自愈（fix-sso-id-default-heal）：库被无约束重建后 CREATE IF
        -- NOT EXISTS 不补既有表的默认值 → INSERT...RETURNING 报 null id（23502）。
        -- 动态扫描 isahl_auth 全部含 id bigint 且无默认值的关系（relkind r/p，
        -- 含 ngac_policy_audit_log 父表与存量分区子表）幂等补 gen_next_zuid()——
        -- ENVIRONMENT_SPEC §6.10 预设 remediation 的进程内 ensure 形态；父表持有
        -- 默认后，后续 CREATE TABLE ... PARTITION OF 新分区自动复制。显式 id 插入
        -- 路径不受列默认值影响。
        DO $iddef$
        DECLARE t record;
        BEGIN
            FOR t IN
                SELECT c.relname
                FROM pg_class c
                JOIN pg_namespace n ON n.oid = c.relnamespace
                JOIN pg_attribute a ON a.attrelid = c.oid
                    AND a.attname = 'id' AND NOT a.attisdropped
                WHERE n.nspname = 'isahl_auth'
                  AND c.relkind IN ('r', 'p')
                  AND format_type(a.atttypid, a.atttypmod) = 'bigint'
                  AND NOT EXISTS (
                      SELECT 1 FROM pg_attrdef d
                      WHERE d.adrelid = c.oid AND d.adnum = a.attnum
                  )
            LOOP
                EXECUTE format(
                    'ALTER TABLE isahl_auth.%I ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid()',
                    t.relname
                );
            END LOOP;
        END
        $iddef$;
        -- SSO 自有审计落库表（F1 管道）；isahl_audit 其余 Gateway 管辖表不批量触碰
        ALTER TABLE IF EXISTS isahl_audit.audit_events
            ALTER COLUMN id SET DEFAULT isahl.gen_next_zuid();"#,
    ))
    .execute(pool)
    .await
    {
        log::warn!("ensure_ngac_extension_tables: auth_users 自愈失败: {}", e);
    }

    // 2. 030 版本信号自愈（ngac_policy_version 兜底 + bump 触发器；幂等迁移
    //    整文件执行——同源零复制）
    const M030: &str = include_str!("../../migrations/030_ensure_ngac_policy_version_seed.sql");
    if let Err(e) = sqlx::raw_sql(sqlx::AssertSqlSafe(M030)).execute(pool).await {
        log::warn!("ensure_ngac_extension_tables: 030 版本信号自愈失败: {}", e);
        EXTENSION_TABLES_ENSURED.store(true, Ordering::Relaxed);
        return;
    }

    // 2.5 指派/委托版本 bump 触发器（fix-ngac-decision-consistency D5；复用 030 的
    //    ngac_bump_policy_version 函数，零复制；DROP IF EXISTS + CREATE 幂等）。
    //    显式不挂 ngac_user_attribute（NGAC_SPEC §2.2.3 禁令）与 ngac_object_attribute
    //    （CRUD 建行自动注册 OA，bump 会导致全 worker 缓存风暴）。
    if let Err(e) = sqlx::raw_sql(sqlx::AssertSqlSafe(
        r#"
        DROP TRIGGER IF EXISTS trg_ngac_user_rr_attribute_version ON isahl_auth.ngac_user_rr_attribute;
        CREATE TRIGGER trg_ngac_user_rr_attribute_version
            AFTER INSERT OR UPDATE OR DELETE ON isahl_auth.ngac_user_rr_attribute
            FOR EACH STATEMENT
            EXECUTE FUNCTION isahl_auth.ngac_bump_policy_version();

        DROP TRIGGER IF EXISTS trg_ngac_delegation_version ON isahl_auth.ngac_delegation;
        CREATE TRIGGER trg_ngac_delegation_version
            AFTER INSERT OR UPDATE OR DELETE ON isahl_auth.ngac_delegation
            FOR EACH STATEMENT
            EXECUTE FUNCTION isahl_auth.ngac_bump_policy_version();
        "#,
    ))
    .execute(pool)
    .await
    {
        log::warn!("ensure_ngac_extension_tables: 指派/委托 bump 触发器自愈失败: {}", e);
    }

    // 3. 核心约束自愈（主键 + 去重 + 唯一索引——防 FK/ON CONFLICT 失败；
    //    与 tests/common/self_heal（auth_users）分工：本块仅 NGAC 表）
    if let Err(e) = ensure_ngac_core_constraints(pool).await {
        log::warn!("ensure_ngac_extension_tables: 核心约束自愈失败: {}", e);
    }

    // 4. 基础 access_right 幂等补齐（Gateway seed ACCESS_RIGHTS 同源清单——
    //    须在唯一索引之后：ON CONFLICT (o_name) 依赖推理目标；test 库重建后
    //    种子 AR 缺失会破坏 matrix/explain 等既有套件）
    for right in [
        "read", "write", "delete", "approve", "admin", "create", "update", "list", "transfer",
        "cc", "withdraw",
    ] {
        if let Err(e) = sqlx::query(
            "INSERT INTO isahl_auth.ngac_access_right (o_name) VALUES ($1) \
             ON CONFLICT (o_name) DO NOTHING",
        )
        .bind(right)
        .execute(pool)
        .await
        {
            log::warn!("ensure_ngac_extension_tables: AR {right} 补齐失败: {}", e);
        }
    }

    EXTENSION_TABLES_ENSURED.store(true, Ordering::Relaxed);
}

/// NGAC 核心表主键/唯一索引/去重自愈（pub(crate)：tests/common 复用同一实现）。
pub(crate) async fn ensure_ngac_core_constraints(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(sqlx::AssertSqlSafe(
        r#"
        -- 主键补齐（FK 引用目标 MUST 有主键；DO 块幂等）
        DO $pk$
        DECLARE t text;
        BEGIN
            FOREACH t IN ARRAY ARRAY[
                'ngac_policy_class', 'ngac_user_attribute', 'ngac_object_attribute',
                'ngac_access_right', 'ngac_association', 'ngac_prohibition',
                'ngac_user_rr_attribute'
            ]
            LOOP
                IF to_regclass(format('isahl_auth.%I', t)) IS NOT NULL AND NOT EXISTS (
                    SELECT 1 FROM pg_constraint
                    WHERE conrelid = format('isahl_auth.%I', t)::regclass AND contype = 'p'
                ) THEN
                    EXECUTE format('ALTER TABLE isahl_auth.%I ADD PRIMARY KEY (id)', t);
                END IF;
            END LOOP;
        END
        $pk$;

        -- 前置去重（无约束期间的多重插入；保留最小 id；deleted 行一并清理——
        -- 全索引 uq_ngac_oa_resource_full 要求全表唯一，软删残留会挡建索引）
        DELETE FROM isahl_auth.ngac_object_attribute a
        USING isahl_auth.ngac_object_attribute b
        WHERE a.id > b.id AND a.resource_type = b.resource_type
          AND a.fk_resource = b.fk_resource;
        DELETE FROM isahl_auth.ngac_policy_class a
        USING isahl_auth.ngac_policy_class b
        WHERE a.id > b.id AND a.o_name = b.o_name;
        DELETE FROM isahl_auth.ngac_user_attribute a
        USING isahl_auth.ngac_user_attribute b
        WHERE a.id > b.id AND a.deleted_at IS NULL AND b.deleted_at IS NULL
          AND a.o_name = b.o_name AND a.fk_policy_class = b.fk_policy_class;
        DELETE FROM isahl_auth.ngac_object_attribute a
        USING isahl_auth.ngac_object_attribute b
        WHERE a.id > b.id AND a.deleted_at IS NULL AND b.deleted_at IS NULL
          AND a.resource_type = b.resource_type AND a.fk_resource = b.fk_resource;
        DELETE FROM isahl_auth.ngac_access_right a
        USING isahl_auth.ngac_access_right b
        WHERE a.id > b.id AND a.o_name = b.o_name;
        DELETE FROM isahl_auth.ngac_user_rr_attribute a
        USING isahl_auth.ngac_user_rr_attribute b
        WHERE a.id > b.id AND a.deleted_at IS NULL AND b.deleted_at IS NULL
          AND a.fk_user = b.fk_user AND a.fk_user_attribute = b.fk_user_attribute;

        -- 唯一索引补齐（与 DB 实测 \d 对齐；idx_ngac_ua_name_pc_unique 是
        -- ON CONFLICT (o_name, fk_policy_class) WHERE (deleted_at IS NULL) 的推理目标）
        CREATE UNIQUE INDEX IF NOT EXISTS idx_ngac_ua_name_pc_unique
            ON isahl_auth.ngac_user_attribute (o_name, fk_policy_class)
            WHERE deleted_at IS NULL;
        CREATE UNIQUE INDEX IF NOT EXISTS ngac_policy_class_o_name_key
            ON isahl_auth.ngac_policy_class (o_name);
        CREATE UNIQUE INDEX IF NOT EXISTS uq_ngac_oa_resource
            ON isahl_auth.ngac_object_attribute (resource_type, fk_resource)
            WHERE deleted_at IS NULL;
        -- 非部分兜底（历史 ON CONFLICT (resource_type, fk_resource) 无谓词写法
        -- 无法推理部分索引 → 42P10 被 .ok() 吞导致行不落；补全索引修复该模式）
        CREATE UNIQUE INDEX IF NOT EXISTS uq_ngac_oa_resource_full
            ON isahl_auth.ngac_object_attribute (resource_type, fk_resource);
        CREATE UNIQUE INDEX IF NOT EXISTS ngac_access_right_o_name_key
            ON isahl_auth.ngac_access_right (o_name);
        CREATE UNIQUE INDEX IF NOT EXISTS ngac_user_rr_attribute_fk_user_fk_user_attribute_key
            ON isahl_auth.ngac_user_rr_attribute (fk_user, fk_user_attribute);

        -- 审计分区兜底（032 同源语义；孤儿处置：0 行 DROP 重建，非空改名保留）
        DO $part$
        DECLARE
            v_parent CONSTANT REGCLASS := 'isahl_auth.ngac_policy_audit_log';
            v_cur_start DATE := date_trunc('month', NOW())::DATE;
            v_next_start DATE := (date_trunc('month', NOW()) + INTERVAL '1 month')::DATE;
            v_next_end DATE := (date_trunc('month', NOW()) + INTERVAL '2 months')::DATE;
            v_names TEXT[] := ARRAY[
                'ngac_policy_audit_log_' || to_char(v_cur_start, 'YYYY_MM'),
                'ngac_policy_audit_log_' || to_char(v_next_start, 'YYYY_MM'),
                'ngac_policy_audit_log_default'
            ];
            v_froms DATE[] := ARRAY[v_cur_start, v_next_start, NULL];
            v_tos DATE[] := ARRAY[v_next_start, v_next_end, NULL];
            v_name TEXT;
            v_rows BIGINT;
        BEGIN
            FOR i IN 1..3 LOOP
                v_name := v_names[i];
                IF to_regclass('isahl_auth.' || v_name) IS NOT NULL THEN
                    IF EXISTS (
                        SELECT 1 FROM pg_inherits
                        WHERE inhrelid = ('isahl_auth.' || v_name)::regclass
                          AND inhparent = v_parent
                    ) THEN
                        CONTINUE;
                    END IF;
                    EXECUTE format('SELECT count(*) FROM isahl_auth.%I', v_name) INTO v_rows;
                    IF v_rows > 0 THEN
                        EXECUTE format('ALTER TABLE isahl_auth.%I RENAME TO %I',
                            v_name, v_name || '_orphan_' || to_char(NOW(), 'YYYYMMDD_HH24MISS'));
                    ELSE
                        EXECUTE format('DROP TABLE isahl_auth.%I', v_name);
                    END IF;
                END IF;
                IF v_froms[i] IS NULL THEN
                    EXECUTE format(
                        'CREATE TABLE isahl_auth.%I PARTITION OF isahl_auth.ngac_policy_audit_log DEFAULT',
                        v_name
                    );
                ELSE
                    EXECUTE format(
                        'CREATE TABLE isahl_auth.%I PARTITION OF isahl_auth.ngac_policy_audit_log FOR VALUES FROM (%L) TO (%L)',
                        v_name, v_froms[i], v_tos[i]
                    );
                END IF;
            END LOOP;
        END
        $part$;
        "#,
    ))
    .execute(pool)
    .await?;
    Ok(())
}
