//! NGAC 扩展表 + 运行时结构幂等自愈（零 DDL 交付收敛，add-ngac-runtime-ensure）
//!
//! 项目零 DDL 交付裁决（remove-ddl-delivery-pipeline）：无 DDL 交付物文件，
//! 基础设施表由代码运行时 ensure 幂等自愈（对齐 Gateway seed
//! `ensure_gateway_seed_self_check` 先例）。本模块自愈范围：
//!
//! 1. **扩展表建表**（业务扩展三张 + D-1 权限规范三张）：
//!    - `ngac_access_request`（权限申请，add-ngac-access-request）
//!    - `ngac_delegation`（通用委托，add-ngac-delegation）
//!    - `ngac_binding_request`（绑定申请，add-ngac-binding-request）
//!    - `org_policy_class` / `org_policy_rule` / `org_policy_label`
//!      （权限规范线，d1-org-policy-assets；state 链 CHECK + 审计列；
//!      label 表 code 主键，label_code 仅存 code 字符串不 FK 冻结域）
//! 2. **030 版本信号自愈**：`ngac_policy_version` 兜底 + bump 触发器
//!    （SQL 为代码内嵌资产 `ngac/sql/030_ngac_policy_version_signal.sql`，随 crate 编译；
//!    无触发器时策略图永不 reload，决策与库内策略脱节）。
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
    //    前三条：业务扩展表；后三条：D-1 权限规范表（org_policy_class/
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
        // NGAC 义务表（`ngac_obligation`，NGAC_SPEC §2.6）：**历史由仓外迁移引入**
        // （2026-09-30 实测：本仓无其 CREATE TABLE；活库形态 = 12 列、**无 PK/索引**）。
        // 此处为 **fresh 库**给出规范形态（PK + 读径部分索引）：否则新库无该表 ⇒
        // 义务求值静默失效（`ngac/obligation.rs` 按空义务 fail-safe）。
        // 已存在库由 `IF NOT EXISTS` 保持原样（存量库补 PK/索引需另立迁移，不在此隐式 ALTER）。
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.ngac_obligation (
            id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
            o_name TEXT,
            event_pattern TEXT,
            response_ops JSONB DEFAULT '[]'::jsonb,
            is_active BOOLEAN DEFAULT TRUE,
            priority INTEGER DEFAULT 0,
            conditions JSONB DEFAULT '{}'::jsonb,
            phase TEXT NOT NULL DEFAULT 'post' CHECK (phase IN ('pre', 'post')),
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            created_by_id BIGINT,
            updated_by_id BIGINT,
            deleted_at TIMESTAMPTZ
        );
        CREATE INDEX IF NOT EXISTS idx_ngac_obligation_active
            ON isahl_auth.ngac_obligation (priority, id) WHERE is_active AND deleted_at IS NULL;
        -- 存量库追加 pre/post 相位列（add-ngac-pre-obligations）：加性，默认 'post' ⇒ 语义不变。
        ALTER TABLE isahl_auth.ngac_obligation
            ADD COLUMN IF NOT EXISTS phase TEXT NOT NULL DEFAULT 'post' CHECK (phase IN ('pre', 'post'))"#,
        // 权限规范（D-1 org-policy-assets；DDL 概念见 ngac-org-phase-d1-spec §2）。
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
        // SoD 冲突对（establish-age-graph-analysis-plane 任务 3.2；规约
        // openspec/specs/ngac-graph-analysis :: sod-conflict-pair-configuration）：
        // UA 级冲突对（无 rights 维——spec delta 口径）；检测 = 同一用户有效 UA
        // 闭包内冲突对两侧（分析端点族 `ngac/analysis.rs` 唯一消费方）。
        r#"CREATE TABLE IF NOT EXISTS isahl_auth.ngac_sod_pair (
            id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
            o_name TEXT NOT NULL,
            fk_ua_a BIGINT NOT NULL REFERENCES isahl_auth.ngac_user_attribute(id) ON DELETE CASCADE,
            fk_ua_b BIGINT NOT NULL REFERENCES isahl_auth.ngac_user_attribute(id) ON DELETE CASCADE,
            is_active BOOLEAN NOT NULL DEFAULT TRUE,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            created_by_id BIGINT,
            updated_by_id BIGINT,
            deleted_at TIMESTAMPTZ,
            CONSTRAINT ck_ngac_sod_pair_distinct CHECK (fk_ua_a <> fk_ua_b)
        );
        CREATE INDEX IF NOT EXISTS idx_ngac_sod_pair_active
            ON isahl_auth.ngac_sod_pair (id) WHERE is_active AND deleted_at IS NULL"#,
        // 死面退役（drop-ngac-dead-schema，2026-10-06 用户裁决「删除全部死面」）：
        // 全仓零读写对象幂等下线——任意存量库（含 pre/prod）下次启动自愈收敛；
        // 恢复源 = Backup DDL 快照 + change design 逆语句。边界：业务根表
        // zc_id_lifecycle 的 ak_* 属权列与 ngac_object_attribute.resource_identifier 不动。
        r#"DROP TABLE IF EXISTS isahl_auth.ngac_resource_identifier;
        ALTER TABLE isahl_auth.ngac_object_attribute DROP COLUMN IF EXISTS ak_permit_user;
        ALTER TABLE isahl_auth.ngac_object_attribute DROP COLUMN IF EXISTS ak_benefit_user;
        ALTER TABLE isahl_auth.ngac_object_attribute DROP COLUMN IF EXISTS ak_access_user;
        ALTER TABLE isahl_auth.ngac_association DROP COLUMN IF EXISTS condition_expr"#,
        // 关联到期列（change add-ngac-grant-revocation-and-expiry）：加性，NULL = 永不过期（语义不变）；
        // 到期由决策侧装载面过滤（`ngac-host::load_policy_graph`）+ 快照到期重载。
        r#"ALTER TABLE isahl_auth.ngac_association
               ADD COLUMN IF NOT EXISTS expires_at TIMESTAMPTZ"#,
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

    // 1.4 账号枚举类型（openspec/specs/auth-users-schema：status/user_type 为原生枚举）。
    //     MUST 先于 1.5 的 auth_users 建表——建表字面直接引用这两个类型。
    if let Err(e) = sqlx::raw_sql(sqlx::AssertSqlSafe(
        r#"DO $enums$
        BEGIN
            -- 并发自愈序列化：多进程（生产 N worker / 测试并行二进制）同时启动时，
            -- check-then-act 的建类型/迁列 MUST 串行，否则竞争 duplicate_object / 42501。
            -- 事务级锁（raw_sql = 单隐式事务）⇒ 语句结束自动释放。
            PERFORM pg_advisory_xact_lock(hashtext('isahl_auth'), hashtext('auth_users'));
            IF NOT EXISTS (
                SELECT 1 FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
                 WHERE n.nspname = 'isahl_auth' AND t.typname = 'auth_user_status'
            ) THEN
                CREATE TYPE isahl_auth.auth_user_status AS ENUM
                    ('active', 'pending', 'pending_approval', 'identity_submitted', 'disabled', 'rejected');
            END IF;
            IF NOT EXISTS (
                SELECT 1 FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace
                 WHERE n.nspname = 'isahl_auth' AND t.typname = 'auth_user_type'
            ) THEN
                CREATE TYPE isahl_auth.auth_user_type AS ENUM
                    ('standard', 'external', 'service', 'system');
            END IF;
        END
        $enums$;"#,
    ))
    .execute(pool)
    .await
    {
        log::warn!("ensure_ngac_extension_tables: 账号枚举类型自愈失败（重启后重试）: {}", e);
    }

    // 1.5 auth_users 幂等自愈（形态 = openspec/specs/auth-users-schema「凭据 + 状态 + 主体绑定」；
    //     name/email/is_ldap_user/ldap_dn 已下线——LDAP 绑定 = ldap jsonb，邮箱唯一存储 =
    //     auth_user_emails，显示名读径 = COALESCE(display_name, username)；
    //     isahl_auth 基础设施表由运行时 ensure 自愈——零 DDL 交付裁决范围）
    if let Err(e) = sqlx::raw_sql(sqlx::AssertSqlSafe(
        r#"SELECT pg_advisory_xact_lock(hashtext('isahl_auth'), hashtext('auth_users'));
        CREATE TABLE IF NOT EXISTS isahl_auth.auth_users (
            id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
            username TEXT UNIQUE,
            password_hash TEXT,
            status isahl_auth.auth_user_status DEFAULT 'active',
            user_type isahl_auth.auth_user_type DEFAULT 'standard',
            last_login_at TIMESTAMPTZ,
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            created_by_id BIGINT,
            updated_by_id BIGINT,
            mfa_secret TEXT,
            mfa_enabled BOOLEAN DEFAULT false,
            mfa_bypass_codes TEXT[],
            is_active BOOLEAN DEFAULT true,
            display_name TEXT,
            ldap JSONB,
            entity_id BIGINT,
            entity_table TEXT,
            fk_policy_class BIGINT REFERENCES isahl_auth.ngac_policy_class (id),
            settings JSONB DEFAULT '{}',
            failed_login_attempts INTEGER NOT NULL DEFAULT 0,
            locked_until TIMESTAMPTZ,
            notification_preferences JSONB NOT NULL DEFAULT '{}',
            subscriptions JSONB NOT NULL DEFAULT '[]'
        );
        -- 邮箱 MUST NOT 全局唯一（allow-duplicate-email-accounts）：同一 email MAY 属于多个账号；
        -- 唯一身份基点 = username（其去重 + 唯一索引保留，见下）。
        -- 历史自愈副作用 MUST NOT 保留：跨账号 email 去重 DELETE 会真实销毁账号行，
        -- email 唯一索引会让约束改动在下次启动被回滚。
        -- 两种形态都要能去唯一：① 独立唯一索引（本 ensure 建的经典形态）→ DROP INDEX；
        -- ② 建表期 inline UNIQUE 的约束持有索引（测试/重建库）→ DROP INDEX 会报
        --    「cannot drop index ... because constraint ... requires it」并**中断整段
        --    raw_sql**（连带跳过 id 默认自愈）⇒ MUST 先按约束名删，再兜底删索引。
        ALTER TABLE isahl_auth.auth_users DROP CONSTRAINT IF EXISTS auth_users_email_key;
        DROP INDEX IF EXISTS isahl_auth.auth_users_email_key;
        DELETE FROM isahl_auth.auth_users a
        USING isahl_auth.auth_users b
        WHERE a.id > b.id AND a.username IS NOT NULL AND a.username = b.username;
        CREATE UNIQUE INDEX IF NOT EXISTS auth_users_username_key
            ON isahl_auth.auth_users (username);
        CREATE INDEX IF NOT EXISTS idx_auth_users_username ON isahl_auth.auth_users(username);
        -- idx_auth_users_email / idx_auth_users_name / idx_auth_users_ldap 均随列下线（见 1.6 迁移块）
        CREATE INDEX IF NOT EXISTS idx_auth_users_active ON isahl_auth.auth_users(is_active);
        -- auth_user_emails：全局 email 唯一 → 账号内 (fk_user, email) 唯一。
        -- to_regclass 守卫：该表由 SSO migrations 034 建立，未迁移库 MUST NOT 阻断本段。
        DO $emails$
        BEGIN
            IF to_regclass('isahl_auth.auth_user_emails') IS NULL THEN
                RETURN;
            END IF;
            DROP INDEX IF EXISTS isahl_auth.uq_auth_user_emails_email;
            DELETE FROM isahl_auth.auth_user_emails a
            USING isahl_auth.auth_user_emails b
            WHERE a.id > b.id AND a.fk_user = b.fk_user AND a.email = b.email
              AND a.deleted_at IS NULL AND b.deleted_at IS NULL;
            CREATE UNIQUE INDEX IF NOT EXISTS uq_auth_user_emails_fk_user_email
                ON isahl_auth.auth_user_emails (fk_user, email) WHERE deleted_at IS NULL;
            CREATE INDEX IF NOT EXISTS idx_auth_user_emails_email
                ON isahl_auth.auth_user_emails (email);
        END
        $emails$;
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

    // 1.6 auth_users 结构性迁移（openspec/specs/auth-users-schema）：未迁移库 → 目标形态。
    //     顺序不变量：回填先于删列；枚举类型/域校验先于列类型转换。
    //     独立 raw_sql（与 1.5 隔离）——域外值 fail-loud 时不连带回滚 1.5 的建表/索引自愈。
    if let Err(e) = sqlx::raw_sql(sqlx::AssertSqlSafe(
        r#"DO $mig$
        DECLARE
            v_bad text;
        BEGIN
            -- 并发自愈序列化（同 1.4）：迁移 DDL 与并发进程的 check-then-act 互斥
            PERFORM pg_advisory_xact_lock(hashtext('isahl_auth'), hashtext('auth_users'));
            -- ① LDAP：收敛到单列 jsonb（先加列，再回填旧两列）
            ALTER TABLE isahl_auth.auth_users ADD COLUMN IF NOT EXISTS ldap JSONB;
            IF EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = 'isahl_auth.auth_users'::regclass
                        AND attname = 'ldap_dn' AND NOT attisdropped) THEN
                UPDATE isahl_auth.auth_users
                   SET ldap = jsonb_build_object('dn', ldap_dn)
                 WHERE ldap IS NULL AND ldap_dn IS NOT NULL;
            END IF;

            -- ② 邮箱：唯一存储 = auth_user_emails（表缺失则建；回填 MUST 先于删列）
            CREATE TABLE IF NOT EXISTS isahl_auth.auth_user_emails (
                id BIGINT PRIMARY KEY DEFAULT isahl.gen_next_zuid(),
                fk_user BIGINT NOT NULL REFERENCES isahl_auth.auth_users (id),
                email TEXT NOT NULL,
                is_primary BOOLEAN NOT NULL DEFAULT FALSE,
                verified BOOLEAN NOT NULL DEFAULT FALSE,
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                deleted_at TIMESTAMPTZ,
                deleted_by_id BIGINT
            );
            CREATE UNIQUE INDEX IF NOT EXISTS uq_auth_user_emails_fk_user_email
                ON isahl_auth.auth_user_emails (fk_user, email) WHERE deleted_at IS NULL;
            CREATE INDEX IF NOT EXISTS idx_auth_user_emails_user ON isahl_auth.auth_user_emails (fk_user);
            CREATE INDEX IF NOT EXISTS idx_auth_user_emails_email ON isahl_auth.auth_user_emails (email);
            IF EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = 'isahl_auth.auth_users'::regclass
                        AND attname = 'email' AND NOT attisdropped) THEN
                INSERT INTO isahl_auth.auth_user_emails (fk_user, email, is_primary)
                SELECT u.id, u.email, TRUE
                  FROM isahl_auth.auth_users u
                 WHERE u.email IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM isahl_auth.auth_user_emails e
                                    WHERE e.fk_user = u.id AND e.email = u.email AND e.deleted_at IS NULL);
            END IF;

            -- ③ status → 原生枚举（类型探测守卫 => 可重放；域外值 fail-loud 待人工裁决）
            IF EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = 'isahl_auth.auth_users'::regclass
                        AND attname = 'status' AND NOT attisdropped)
               AND (SELECT atttypid FROM pg_attribute
                     WHERE attrelid = 'isahl_auth.auth_users'::regclass
                       AND attname = 'status' AND NOT attisdropped)
                   <> 'isahl_auth.auth_user_status'::regtype THEN
                SELECT string_agg(DISTINCT status::text, ',') INTO v_bad
                  FROM isahl_auth.auth_users
                 WHERE status IS NOT NULL
                   AND status::text NOT IN ('active', 'pending', 'pending_approval',
                                            'identity_submitted', 'disabled', 'rejected');
                IF v_bad IS NOT NULL THEN
                    RAISE EXCEPTION 'auth_users.status 存在枚举域外值（%）——MUST 人工裁决后再迁移', v_bad;
                END IF;
                ALTER TABLE isahl_auth.auth_users ALTER COLUMN status DROP DEFAULT;
                ALTER TABLE isahl_auth.auth_users
                    ALTER COLUMN status TYPE isahl_auth.auth_user_status
                    USING status::text::isahl_auth.auth_user_status;
                ALTER TABLE isahl_auth.auth_users
                    ALTER COLUMN status SET DEFAULT 'active'::isahl_auth.auth_user_status;
            END IF;

            -- ③′ user_type → 原生枚举（同上）
            IF EXISTS (SELECT 1 FROM pg_attribute WHERE attrelid = 'isahl_auth.auth_users'::regclass
                        AND attname = 'user_type' AND NOT attisdropped)
               AND (SELECT atttypid FROM pg_attribute
                     WHERE attrelid = 'isahl_auth.auth_users'::regclass
                       AND attname = 'user_type' AND NOT attisdropped)
                   <> 'isahl_auth.auth_user_type'::regtype THEN
                SELECT string_agg(DISTINCT user_type::text, ',') INTO v_bad
                  FROM isahl_auth.auth_users
                 WHERE user_type IS NOT NULL
                   AND user_type::text NOT IN ('standard', 'external', 'service', 'system');
                IF v_bad IS NOT NULL THEN
                    RAISE EXCEPTION 'auth_users.user_type 存在枚举域外值（%）——MUST 人工裁决后再迁移', v_bad;
                END IF;
                ALTER TABLE isahl_auth.auth_users ALTER COLUMN user_type DROP DEFAULT;
                ALTER TABLE isahl_auth.auth_users
                    ALTER COLUMN user_type TYPE isahl_auth.auth_user_type
                    USING user_type::text::isahl_auth.auth_user_type;
                ALTER TABLE isahl_auth.auth_users
                    ALTER COLUMN user_type SET DEFAULT 'standard'::isahl_auth.auth_user_type;
            END IF;

            -- ④ 旧列与失效索引下线（回填已完成；DROP COLUMN 连带其唯一约束/索引）
            DROP INDEX IF EXISTS isahl_auth.idx_auth_users_name;
            DROP INDEX IF EXISTS isahl_auth.idx_auth_users_email;
            DROP INDEX IF EXISTS isahl_auth.idx_auth_users_ldap;
            ALTER TABLE isahl_auth.auth_users DROP COLUMN IF EXISTS name;
            ALTER TABLE isahl_auth.auth_users DROP COLUMN IF EXISTS email;
            ALTER TABLE isahl_auth.auth_users DROP COLUMN IF EXISTS is_ldap_user;
            ALTER TABLE isahl_auth.auth_users DROP COLUMN IF EXISTS ldap_dn;

            -- ⑤ LDAP 存在性部分索引（等价替换旧 idx_auth_users_ldap 的语义）
            CREATE INDEX IF NOT EXISTS idx_auth_users_ldap
                ON isahl_auth.auth_users (ldap) WHERE ldap IS NOT NULL;
        END
        $mig$;"#,
    ))
    .execute(pool)
    .await
    {
        log::warn!("ensure_ngac_extension_tables: auth_users 结构性迁移失败（重启后重试）: {}", e);
    }

    // 2. 030 版本信号自愈（ngac_policy_version 兜底 + bump 触发器；SQL 为代码内嵌资产
    //    `ngac/sql/`，随 crate 编译，无编号排序与记账语义）
    const M030: &str = include_str!("sql/030_ngac_policy_version_signal.sql");
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
        -- (resource_type, fk_resource) 的唯一性由**约束** `uq_ngac_oa_resource`（全列、无谓词）承载：
        --   · 该名在 DB 中为 UNIQUE CONSTRAINT（参考库结构持有，与集成测试同源），
        --     其支撑索引占用同名关系 ⇒ 此前此处带 `WHERE deleted_at IS NULL` 的
        --     `CREATE UNIQUE INDEX IF NOT EXISTS uq_ngac_oa_resource` 恒为 no-op（部分索引从未建成），
        --     该无效语句已删除；约束缺失的库（只跑迁移子集）由下方 DO 块补建同定义索引。
        --   · 全列唯一是 `ON CONFLICT (resource_type, fk_resource)` 的推理目标（无谓词才可推理）；
        --     软删残留行会占用该唯一键，故上方「保留最小 id」的去重 DELETE 是必要前置。
        DO $oa_uniq$
        BEGIN
            IF NOT EXISTS (
                SELECT 1 FROM pg_constraint
                WHERE conrelid = 'isahl_auth.ngac_object_attribute'::regclass
                  AND conname = 'uq_ngac_oa_resource'
            ) THEN
                CREATE UNIQUE INDEX IF NOT EXISTS uq_ngac_oa_resource
                    ON isahl_auth.ngac_object_attribute (resource_type, fk_resource);
            END IF;
        END
        $oa_uniq$;
        -- 冗余索引幂等清除：`uq_ngac_oa_resource_full` 的列与谓词与 `uq_ngac_oa_resource`
        -- 完全相同（同一唯一性两份索引维护），保留前者（约束/推理目标）即可。
        DROP INDEX IF EXISTS isahl_auth.uq_ngac_oa_resource_full;
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
