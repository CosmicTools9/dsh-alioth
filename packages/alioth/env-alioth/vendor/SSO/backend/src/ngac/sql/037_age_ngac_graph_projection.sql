-- 037_age_ngac_graph_projection.sql
-- NGAC AGE 图物化投影（change add-age-graph-projection A-1b，design D1/D2/D9）
--
-- 幂等重放安全；AGE 扩展缺失时全部 no-op（部署镜像缺环不阻断，design D9）。
-- 关系表 ngac_* 永远是唯一真相源——AGE 图为只读投影，唯一写入方 = 本文件的
-- 同步触发器与 age_rebuild_ngac_graph()。
--
-- 环境事实（2026-09-18 实测，AGE 1.8.0 + PG18.6）：
-- - create_graph 在既有 schema 上必败（CREATE SCHEMA 冲突）——图引导走
--   「手工注册 ag_graph + 官方 create_vlabel/create_elabel」路径；
-- - create_*label 内部索引 DDL 用未限定名 graphid_ops——MUST 在 search_path
--   含 ag_catalog 的会话下执行（zh 错误消息参数序会误导为「方法/类错位」）；
-- - ag_catalog 被 pg_dump --exclude-schema 排除 → restore 后图注册丢失；带回的
--   `_ag_label_*`（含数据）由引导**非阻断清理 + 重建覆盖**（2026-09-20 修正：
--   残留不得抛错阻断，投影为派生数据、rebuild + 对账为权威收敛路径）。

-- ═══ 0) 孤儿函数退役（design D6）═══
DROP FUNCTION IF EXISTS isahl_auth.cypher_user_hierarchy_template();

-- ═══ 1) 图注册一致性 + 可用性守卫（触发器/重建/引导共用；不一致 → false → 调用方 no-op）═══
-- 就绪判据 MUST 覆盖三条：扩展可用、注册指向当前 schema、期望 label 全部注册且其关系真实存在。
-- 只按名判「注册行存在」不足：整库重建（DROP SCHEMA + schema-only 恢复）不触碰 ag_catalog，
-- 旧注册行会存活并指向已死 schema OID、label 关系随 schema 消失 ⇒ 触发器会对不存在的关系
-- 执行 cypher()，而 AGE C 层段错误**不可被 plpgsql EXCEPTION 捕获**——2026-09-20 事故：
-- 整库重建后的 NGAC 种子把 PG 后端打成 SIGSEGV（实例 crash + recovery + 重建管线报废）。
-- 期望 label 清单 = §3 创建清单的单一来源（改清单 MUST 同步改此处）。
CREATE OR REPLACE FUNCTION isahl_auth.age_ngac_graph_registered()
RETURNS boolean
LANGUAGE plpgsql STABLE
AS $fn$
BEGIN
    RETURN EXISTS (
        SELECT 1 FROM ag_catalog.ag_graph g
         WHERE g.name = 'isahl_auth'
           AND g.namespace = to_regnamespace('isahl_auth')
           AND NOT EXISTS (
                 SELECT 1
                   FROM unnest(ARRAY['_ag_label_vertex', '_ag_label_edge', 'User', 'UserAttribute',
                                     'ObjectAttribute', 'HAS_ATTRIBUTE', 'PARENT_OF',
                                     'ASSOCIATION', 'PROHIBITION']) AS expected(lname)
                  WHERE NOT EXISTS (
                        SELECT 1 FROM ag_catalog.ag_label l
                         WHERE l.graph = g.graphid
                           AND l.name = expected.lname
                           AND EXISTS (SELECT 1 FROM pg_class c WHERE c.oid = l.relation))));
EXCEPTION WHEN OTHERS THEN
    RETURN false;
END $fn$;

CREATE OR REPLACE FUNCTION isahl_auth.age_ngac_graph_ready()
RETURNS boolean
LANGUAGE plpgsql STABLE
AS $fn$
BEGIN
    RETURN EXISTS(SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'age')
       AND isahl_auth.age_ngac_graph_registered();
EXCEPTION WHEN OTHERS THEN
    RETURN false;
END $fn$;

-- ═══ 2) 图引导（幂等）：残留清理 → label id 序列 → ag_graph 注册 ═══
DO $bootstrap$
DECLARE
    orphan_row_cnt bigint;
BEGIN
    IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'age') THEN
        RAISE NOTICE 'age 扩展未安装，跳过 NGAC 图引导';
        RETURN;
    END IF;
    -- 幂等早退判据 = 注册一致性（**非**「按名存在」）：namespace 对齐当前 schema 且 label 关系齐备才跳过。
    IF isahl_auth.age_ngac_graph_registered() THEN
        RETURN; -- 已注册且一致，幂等
    END IF;

    -- 悬挂/失效注册清理（仅限本图；ag_label 先于 ag_graph——后者是前者的外键目标）。
    -- 触发条件严格：按名存在但 namespace 不指向当前 schema，或期望 label 缺失/其关系已消失
    -- ——典型来源 = 整库重建 DROP SCHEMA 后 ag_catalog 里的旧注册存活（2026-09-20 事故）。
    -- 不清理则下方 label 注册会撞 fk_graph_oid（graph=新 OID 不在 ag_graph），
    -- 而 psql -f 在无 ON_ERROR_STOP 时静默通过 → 十步之后由 NGAC 写入击穿 PG。
    DELETE FROM ag_catalog.ag_label
     WHERE graph IN (SELECT graphid FROM ag_catalog.ag_graph WHERE name = 'isahl_auth');
    DELETE FROM ag_catalog.ag_graph WHERE name = 'isahl_auth';

    -- 残留处理（2026-09-20 修正，restore 自愈实测）：本图是**派生投影**（`ngac_*` 关系表为
    -- 权威源，重建可自愈），故残留一律「先解锁 → 重建覆盖」，MUST NOT 抛错阻断自愈。
    -- 三处历史缺陷（实证）：
    --   ① 无 `to_regclass` 守卫 ⇒ 空库态（表不存在，如全新部署）直接 `relation does not exist` 中止；
    --   ② `count(*)` 经继承计入子表（`User`/`HAS_ATTRIBUTE` 等正常图数据）⇒ restore 态被误判为
    --      「危险残留」而 RAISE EXCEPTION（实测 1933 行）⇒ 整批迁移中止，注册不恢复；
    --   ③ `DROP TABLE` 无 `CASCADE` ⇒ 子表存在时 `cannot drop … because other objects depend on it`。
    IF to_regclass('isahl_auth._ag_label_vertex') IS NOT NULL
       OR to_regclass('isahl_auth._ag_label_edge') IS NOT NULL THEN
        SELECT COALESCE((SELECT count(*) FROM isahl_auth._ag_label_vertex), 0) INTO orphan_row_cnt;
        IF orphan_row_cnt > 0 THEN
            RAISE NOTICE 'isahl_auth：检测到既有投影数据 % 行（注册缺失或漂移）——按幂等重建覆盖', orphan_row_cnt;
        END IF;
        DROP TABLE IF EXISTS isahl_auth._ag_label_vertex CASCADE;
        DROP TABLE IF EXISTS isahl_auth._ag_label_edge CASCADE;
        DROP SEQUENCE IF EXISTS isahl_auth._ag_label_vertex_id_seq;
        DROP SEQUENCE IF EXISTS isahl_auth._ag_label_edge_id_seq;
        DROP SEQUENCE IF EXISTS isahl_auth._label_id_seq;
    END IF;
    -- 图级 label id 序列（对齐 AGE create_schema_for_graph：int4 / MAX 65535 / CYCLE）
    CREATE SEQUENCE isahl_auth._label_id_seq AS integer MAXVALUE 65535 CYCLE;

    -- 注册图（graphid = namespace oid——确定性、天然唯一，restore 后幂等复现）
    INSERT INTO ag_catalog.ag_graph (graphid, name, namespace)
    VALUES ('isahl_auth'::regnamespace::oid, 'isahl_auth', 'isahl_auth'::regnamespace);
END
$bootstrap$;

-- ═══ 3) Label 创建（官方 C 路径；search_path 必含 ag_catalog）═══
-- 纯 SQL 手工创建（零 C 调用）：create_vlabel 以 _ag_label_vertex 为 INHERITS 父表，
-- 而基础 label 仅 create_graph 能建（既有 schema 上 create_graph 必败）——手工注册的图
-- 上调用 create_vlabel 会段错误（实测 2026-09-18）。形态对齐官方 pg_dump 实证：
-- 表 + seq(2^48-1) + OWNED BY + pkey + ag_label 行 + id 默认值（默认值依赖 ag_label 行，
-- 必须后置）。全部对象名显式限定，无 search_path 依赖。
DO $labels$
DECLARE
    goid oid := 'isahl_auth'::regnamespace::oid;
    lname text;
    seq_name text;
BEGIN
    IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'age') THEN
        RETURN;
    END IF;
    IF NOT EXISTS(SELECT 1 FROM ag_catalog.ag_graph WHERE name = 'isahl_auth') THEN
        RAISE NOTICE '图未注册，跳过 label 创建';
        RETURN;
    END IF;

    -- ── 基础 vertex label ──
    CREATE TABLE IF NOT EXISTS isahl_auth._ag_label_vertex (
        id ag_catalog.graphid NOT NULL,
        properties ag_catalog.agtype DEFAULT ag_catalog.agtype_build_map() NOT NULL
    );
    CREATE SEQUENCE IF NOT EXISTS isahl_auth._ag_label_vertex_id_seq
        START WITH 1 INCREMENT BY 1 NO MINVALUE MAXVALUE 281474976710655 CACHE 1;
    ALTER SEQUENCE isahl_auth._ag_label_vertex_id_seq OWNED BY isahl_auth._ag_label_vertex.id;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint c JOIN pg_class t ON t.oid = c.conrelid
                    JOIN pg_namespace n ON n.oid = t.relnamespace
                   WHERE n.nspname = 'isahl_auth' AND t.relname = '_ag_label_vertex'
                     AND c.conname = '_ag_label_vertex_pkey') THEN
        ALTER TABLE isahl_auth._ag_label_vertex ADD CONSTRAINT _ag_label_vertex_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM ag_catalog.ag_label WHERE graph = goid AND name = '_ag_label_vertex') THEN
        INSERT INTO ag_catalog.ag_label (name, graph, id, kind, relation, seq_name)
        VALUES ('_ag_label_vertex', goid, nextval('isahl_auth._label_id_seq'), 'v'::ag_catalog.label_kind,
                'isahl_auth._ag_label_vertex'::regclass, '_ag_label_vertex_id_seq');
    END IF;
    ALTER TABLE isahl_auth._ag_label_vertex ALTER COLUMN id SET DEFAULT
        ag_catalog._graphid((ag_catalog._label_id('isahl_auth'::name, '_ag_label_vertex'::name))::integer,
                             nextval('isahl_auth._ag_label_vertex_id_seq'::regclass));

    -- ── 基础 edge label ──
    CREATE TABLE IF NOT EXISTS isahl_auth._ag_label_edge (
        id ag_catalog.graphid NOT NULL,
        start_id ag_catalog.graphid NOT NULL,
        end_id ag_catalog.graphid NOT NULL,
        properties ag_catalog.agtype DEFAULT ag_catalog.agtype_build_map() NOT NULL
    );
    CREATE INDEX IF NOT EXISTS _ag_label_edge_start_id_idx
        ON isahl_auth._ag_label_edge USING btree (start_id ag_catalog.graphid_ops);
    CREATE INDEX IF NOT EXISTS _ag_label_edge_end_id_idx
        ON isahl_auth._ag_label_edge USING btree (end_id ag_catalog.graphid_ops);
    CREATE SEQUENCE IF NOT EXISTS isahl_auth._ag_label_edge_id_seq
        START WITH 1 INCREMENT BY 1 NO MINVALUE MAXVALUE 281474976710655 CACHE 1;
    ALTER SEQUENCE isahl_auth._ag_label_edge_id_seq OWNED BY isahl_auth._ag_label_edge.id;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint c JOIN pg_class t ON t.oid = c.conrelid
                    JOIN pg_namespace n ON n.oid = t.relnamespace
                   WHERE n.nspname = 'isahl_auth' AND t.relname = '_ag_label_edge'
                     AND c.conname = '_ag_label_edge_pkey') THEN
        ALTER TABLE isahl_auth._ag_label_edge ADD CONSTRAINT _ag_label_edge_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM ag_catalog.ag_label WHERE graph = goid AND name = '_ag_label_edge') THEN
        INSERT INTO ag_catalog.ag_label (name, graph, id, kind, relation, seq_name)
        VALUES ('_ag_label_edge', goid, nextval('isahl_auth._label_id_seq'), 'e'::ag_catalog.label_kind,
                'isahl_auth._ag_label_edge'::regclass, '_ag_label_edge_id_seq');
    END IF;
    ALTER TABLE isahl_auth._ag_label_edge ALTER COLUMN id SET DEFAULT
        ag_catalog._graphid((ag_catalog._label_id('isahl_auth'::name, '_ag_label_edge'::name))::integer,
                             nextval('isahl_auth._ag_label_edge_id_seq'::regclass));

    -- ── 业务 vertex labels：INHERITS 基础表 ──
    FOREACH lname IN ARRAY ARRAY['User', 'UserAttribute', 'ObjectAttribute'] LOOP
        seq_name := lname || '_id_seq';
        EXECUTE format('CREATE TABLE IF NOT EXISTS isahl_auth.%I () INHERITS (isahl_auth._ag_label_vertex)', lname);
        EXECUTE format('CREATE SEQUENCE IF NOT EXISTS isahl_auth.%I START WITH 1 INCREMENT BY 1 NO MINVALUE MAXVALUE 281474976710655 CACHE 1', seq_name);
        EXECUTE format('ALTER SEQUENCE isahl_auth.%I OWNED BY isahl_auth.%I.id', seq_name, lname);
        IF NOT EXISTS (SELECT 1 FROM pg_constraint c JOIN pg_class t ON t.oid = c.conrelid
                        JOIN pg_namespace n ON n.oid = t.relnamespace
                       WHERE n.nspname = 'isahl_auth' AND t.relname = lname
                         AND c.conname = lname || '_pkey') THEN
            EXECUTE format('ALTER TABLE isahl_auth.%I ADD CONSTRAINT %I PRIMARY KEY (id)', lname, lname || '_pkey');
        END IF;
        IF NOT EXISTS (SELECT 1 FROM ag_catalog.ag_label WHERE graph = goid AND name = lname) THEN
            INSERT INTO ag_catalog.ag_label (name, graph, id, kind, relation, seq_name)
            VALUES (lname, goid, nextval('isahl_auth._label_id_seq'), 'v'::ag_catalog.label_kind,
                    format('isahl_auth.%I', lname)::regclass, seq_name);
        END IF;
        EXECUTE format('ALTER TABLE isahl_auth.%I ALTER COLUMN id SET DEFAULT ag_catalog._graphid((ag_catalog._label_id(''isahl_auth''::name, %L::name))::integer, nextval(%L::regclass))',
                        lname, lname, format('isahl_auth.%I', seq_name));
    END LOOP;

    -- ── 业务 edge labels：INHERITS 基础边表 ──
    FOREACH lname IN ARRAY ARRAY['HAS_ATTRIBUTE', 'PARENT_OF', 'ASSOCIATION', 'PROHIBITION'] LOOP
        seq_name := lname || '_id_seq';
        EXECUTE format('CREATE TABLE IF NOT EXISTS isahl_auth.%I () INHERITS (isahl_auth._ag_label_edge)', lname);
        EXECUTE format('CREATE SEQUENCE IF NOT EXISTS isahl_auth.%I START WITH 1 INCREMENT BY 1 NO MINVALUE MAXVALUE 281474976710655 CACHE 1', seq_name);
        EXECUTE format('ALTER SEQUENCE isahl_auth.%I OWNED BY isahl_auth.%I.id', seq_name, lname);
        IF NOT EXISTS (SELECT 1 FROM pg_constraint c JOIN pg_class t ON t.oid = c.conrelid
                        JOIN pg_namespace n ON n.oid = t.relnamespace
                       WHERE n.nspname = 'isahl_auth' AND t.relname = lname
                         AND c.conname = lname || '_pkey') THEN
            EXECUTE format('ALTER TABLE isahl_auth.%I ADD CONSTRAINT %I PRIMARY KEY (id)', lname, lname || '_pkey');
        END IF;
        IF NOT EXISTS (SELECT 1 FROM ag_catalog.ag_label WHERE graph = goid AND name = lname) THEN
            INSERT INTO ag_catalog.ag_label (name, graph, id, kind, relation, seq_name)
            VALUES (lname, goid, nextval('isahl_auth._label_id_seq'), 'e'::ag_catalog.label_kind,
                    format('isahl_auth.%I', lname)::regclass, seq_name);
        END IF;
        EXECUTE format('ALTER TABLE isahl_auth.%I ALTER COLUMN id SET DEFAULT ag_catalog._graphid((ag_catalog._label_id(''isahl_auth''::name, %L::name))::integer, nextval(%L::regclass))',
                        lname, lname, format('isahl_auth.%I', seq_name));
    END LOOP;
END
$labels$;

-- ═══ 4) 同步触发器（五表；投影 fail-open：异常只 WARNING，不阻断主写）═══
-- AGE cypher() 解析约束（实测，design D10 基线）：
--   ① 查询串必须是 dollar-quoted 字面量（单引号字面量报「a dollar-quoted string
--      constant is expected」）——经 $cy$...$cy$ 嵌入 format(%s)；
--   ② 第三参必须是裸参数节点（不接受 cast 表达式）——cast 放 USING 值侧
--      （(json::text)::ag_catalog.agtype）；
--   ③ 参数以 $key 直接引用（第三参 map 的键即参数名）。
-- 软删分流：UPDATE 置 deleted_at 非空 → 图侧 DETACH DELETE（表侧对账口径排除
-- 软删行；PDP 图加载同口径过滤——软删边/点 MUST NOT 参与投影）。
-- 运行时值全部经参数通道，Cypher 文本为编译期常量。

CREATE OR REPLACE FUNCTION isahl_auth.age_sync_user_attribute()
RETURNS trigger LANGUAGE plpgsql
AS $fn$
DECLARE
    v_cypher text;
BEGIN
    IF pg_trigger_depth() > 1 OR NOT isahl_auth.age_ngac_graph_ready() THEN
        RETURN NULL;
    END IF;
    -- AGE 生成计划的运算符解析（= / @> 等）要求 search_path 含 ag_catalog（实测
    -- DELETE 路径缺此步报「操作符不存在 agtype @> agtype」）；LOCAL 限本事务。
    PERFORM set_config('search_path', 'ag_catalog, pg_temp', true);
    BEGIN
        IF TG_OP = 'DELETE' OR NEW.deleted_at IS NOT NULL THEN
            v_cypher := 'MATCH (n:UserAttribute {id: $id}) DETACH DELETE n';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object('id', COALESCE(NEW.id, OLD.id))::text)::ag_catalog.agtype;
        ELSE
            v_cypher := 'MERGE (n:UserAttribute {id: $id})
                         SET n.o_name = $o_name, n.fk_policy_class = $fk_policy_class';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object(
                'id', COALESCE(NEW.id, 0),
                'o_name', NEW.o_name,
                'fk_policy_class', NEW.fk_policy_class)::text)::ag_catalog.agtype;
        END IF;
    EXCEPTION WHEN OTHERS THEN
        RAISE WARNING 'age_sync_user_attribute 投影失败（%/%）: %', TG_OP, OLD.id, SQLERRM;
    END;
    RETURN NULL;
END $fn$;
DROP TRIGGER IF EXISTS ngac_age_sync_ua ON isahl_auth.ngac_user_attribute;
CREATE TRIGGER ngac_age_sync_ua
    AFTER INSERT OR UPDATE OR DELETE ON isahl_auth.ngac_user_attribute
    FOR EACH ROW EXECUTE FUNCTION isahl_auth.age_sync_user_attribute();

CREATE OR REPLACE FUNCTION isahl_auth.age_sync_object_attribute()
RETURNS trigger LANGUAGE plpgsql
AS $fn$
DECLARE
    v_cypher text;
BEGIN
    IF pg_trigger_depth() > 1 OR NOT isahl_auth.age_ngac_graph_ready() THEN
        RETURN NULL;
    END IF;
    PERFORM set_config('search_path', 'ag_catalog, pg_temp', true);
    BEGIN
        IF TG_OP = 'DELETE' OR NEW.deleted_at IS NOT NULL THEN
            v_cypher := 'MATCH (n:ObjectAttribute {id: $id}) DETACH DELETE n';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object('id', COALESCE(NEW.id, OLD.id))::text)::ag_catalog.agtype;
        ELSE
            v_cypher := 'MERGE (n:ObjectAttribute {id: $id})
                         SET n.o_name = $o_name, n.resource_type = $resource_type,
                             n.fk_resource = $fk_resource,
                             n.resource_identifier = $resource_identifier';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object(
                'id', COALESCE(NEW.id, 0),
                'o_name', NEW.o_name,
                'resource_type', NEW.resource_type,
                'fk_resource', NEW.fk_resource,
                'resource_identifier', NEW.resource_identifier)::text)::ag_catalog.agtype;
        END IF;
    EXCEPTION WHEN OTHERS THEN
        RAISE WARNING 'age_sync_object_attribute 投影失败（%/%）: %', TG_OP, OLD.id, SQLERRM;
    END;
    RETURN NULL;
END $fn$;
DROP TRIGGER IF EXISTS ngac_age_sync_oa ON isahl_auth.ngac_object_attribute;
CREATE TRIGGER ngac_age_sync_oa
    AFTER INSERT OR UPDATE OR DELETE ON isahl_auth.ngac_object_attribute
    FOR EACH ROW EXECUTE FUNCTION isahl_auth.age_sync_object_attribute();

CREATE OR REPLACE FUNCTION isahl_auth.age_sync_user_rr_attribute()
RETURNS trigger LANGUAGE plpgsql
AS $fn$
DECLARE
    v_cypher text;
BEGIN
    IF pg_trigger_depth() > 1 OR NOT isahl_auth.age_ngac_graph_ready() THEN
        RETURN NULL;
    END IF;
    PERFORM set_config('search_path', 'ag_catalog, pg_temp', true);
    BEGIN
        IF TG_OP = 'DELETE' OR NEW.deleted_at IS NOT NULL
           OR (NEW.expires_at IS NOT NULL AND NEW.expires_at <= now()) THEN
            v_cypher := 'MATCH ()-[r:HAS_ATTRIBUTE {row_id: $row_id}]->() DELETE r';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object('row_id', COALESCE(NEW.id, OLD.id))::text)::ag_catalog.agtype;
        ELSE
            -- 先确保 User 顶点（MERGE 幂等），再连边（端点缺失则 MATCH 落空 = 投影滞后，rebuild 自愈）
            v_cypher := 'MERGE (u:User {id: $fk_user})';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object('fk_user', NEW.fk_user)::text)::ag_catalog.agtype;

            v_cypher := 'MATCH (u:User {id: $fk_user}), (ua:UserAttribute {id: $fk_ua})
                         MERGE (u)-[r:HAS_ATTRIBUTE {row_id: $row_id}]->(ua)
                         SET r.expires_at = $expires_at';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object(
                'fk_user', NEW.fk_user,
                'fk_ua', NEW.fk_user_attribute,
                'row_id', NEW.id,
                'expires_at', NEW.expires_at)::text)::ag_catalog.agtype;
        END IF;
    EXCEPTION WHEN OTHERS THEN
        RAISE WARNING 'age_sync_user_rr_attribute 投影失败（%/%）: %', TG_OP, OLD.id, SQLERRM;
    END;
    RETURN NULL;
END $fn$;
DROP TRIGGER IF EXISTS ngac_age_sync_ua_assign ON isahl_auth.ngac_user_rr_attribute;
CREATE TRIGGER ngac_age_sync_ua_assign
    AFTER INSERT OR UPDATE OR DELETE ON isahl_auth.ngac_user_rr_attribute
    FOR EACH ROW EXECUTE FUNCTION isahl_auth.age_sync_user_rr_attribute();

CREATE OR REPLACE FUNCTION isahl_auth.age_sync_association()
RETURNS trigger LANGUAGE plpgsql
AS $fn$
DECLARE
    v_cypher text;
BEGIN
    IF pg_trigger_depth() > 1 OR NOT isahl_auth.age_ngac_graph_ready() THEN
        RETURN NULL;
    END IF;
    PERFORM set_config('search_path', 'ag_catalog, pg_temp', true);
    BEGIN
        IF TG_OP = 'DELETE' OR NEW.deleted_at IS NOT NULL THEN
            v_cypher := 'MATCH ()-[r:ASSOCIATION {id: $row_id}]->() DELETE r';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object('row_id', COALESCE(NEW.id, OLD.id))::text)::ag_catalog.agtype;
        ELSE
            v_cypher := 'MATCH (ua:UserAttribute {id: $fk_ua}), (oa:ObjectAttribute {id: $fk_oa})
                         MERGE (ua)-[r:ASSOCIATION {id: $row_id}]->(oa)
                         SET r.rights = $rights, r.conditions = $conditions';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object(
                'fk_ua', NEW.fk_user_attribute,
                'fk_oa', NEW.fk_object_attribute,
                'row_id', NEW.id,
                'rights', COALESCE(NEW.ak_access_rights, ARRAY[]::bigint[]),
                'conditions', COALESCE(NEW.conditions, '{}'::jsonb))::text)::ag_catalog.agtype;
        END IF;
    EXCEPTION WHEN OTHERS THEN
        RAISE WARNING 'age_sync_association 投影失败（%/%）: %', TG_OP, OLD.id, SQLERRM;
    END;
    RETURN NULL;
END $fn$;
DROP TRIGGER IF EXISTS ngac_age_sync_assoc ON isahl_auth.ngac_association;
CREATE TRIGGER ngac_age_sync_assoc
    AFTER INSERT OR UPDATE OR DELETE ON isahl_auth.ngac_association
    FOR EACH ROW EXECUTE FUNCTION isahl_auth.age_sync_association();

CREATE OR REPLACE FUNCTION isahl_auth.age_sync_prohibition()
RETURNS trigger LANGUAGE plpgsql
AS $fn$
DECLARE
    v_cypher text;
BEGIN
    IF pg_trigger_depth() > 1 OR NOT isahl_auth.age_ngac_graph_ready() THEN
        RETURN NULL;
    END IF;
    PERFORM set_config('search_path', 'ag_catalog, pg_temp', true);
    BEGIN
        IF TG_OP = 'DELETE' OR NEW.deleted_at IS NOT NULL THEN
            v_cypher := 'MATCH ()-[r:PROHIBITION {id: $row_id}]->() DELETE r';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object('row_id', COALESCE(NEW.id, OLD.id))::text)::ag_catalog.agtype;
        ELSE
            v_cypher := 'MATCH (ua:UserAttribute {id: $fk_ua}), (oa:ObjectAttribute {id: $fk_oa})
                         MERGE (ua)-[r:PROHIBITION {id: $row_id}]->(oa)
                         SET r.rights = $rights, r.is_active = $is_active, r.conditions = $conditions';
            EXECUTE format('SELECT 1 FROM ag_catalog.cypher(''isahl_auth'', $cy$ %s $cy$, $1) AS (r ag_catalog.agtype)', v_cypher)
            USING (json_build_object(
                'fk_ua', NEW.fk_user_attribute,
                'fk_oa', NEW.fk_object_attribute,
                'row_id', NEW.id,
                'rights', COALESCE(NEW.ak_access_rights, ARRAY[]::bigint[]),
                'is_active', COALESCE(NEW.is_active, true),
                'conditions', COALESCE(NEW.conditions, '{}'::jsonb))::text)::ag_catalog.agtype;
        END IF;
    EXCEPTION WHEN OTHERS THEN
        RAISE WARNING 'age_sync_prohibition 投影失败（%/%）: %', TG_OP, OLD.id, SQLERRM;
    END;
    RETURN NULL;
END $fn$;
DROP TRIGGER IF EXISTS ngac_age_sync_prohib ON isahl_auth.ngac_prohibition;
CREATE TRIGGER ngac_age_sync_prohib
    AFTER INSERT OR UPDATE OR DELETE ON isahl_auth.ngac_prohibition
    FOR EACH ROW EXECUTE FUNCTION isahl_auth.age_sync_prohibition();

-- ═══ 5) 全量重建（幂等；restore / 断同步后的对账自愈通道）═══
-- O(n) 直插，不走 Cypher（UNWIND+MERGE 按属性匹配 = 每行全表扫，17k×18k 实测 O(n²) 不可用）。
-- 顶点 graphid = _graphid(label_id, rank)：rank = row_number() OVER (ORDER BY 行 id)——
-- 确定性且 ≤ 行数（≪ 2^48，规避 zuid 量级行 id 超 graphid entry 位宽的问题）。
-- TRUNCATE RESTART IDENTITY 后将各 label 序列 setval 对齐 max(rank)，防触发器
-- 后续 MERGE 新顶点与重建顶点撞 id。触发器侧单行 MERGE 仍走 Cypher。
CREATE OR REPLACE FUNCTION isahl_auth.age_rebuild_ngac_graph()
RETURNS void LANGUAGE plpgsql
AS $fn$
DECLARE
    -- 标签 id 延迟到守卫之后取（D9：无 AGE 环境函数入口即解析 ag_catalog._label_id
    -- 会撞 schema does not exist；plpgsql 表达式首次执行才规划，守卫早退后永不触达）
    lbl_ua integer;
    lbl_oa integer;
    lbl_user integer;
    max_rnk bigint;
BEGIN
    IF NOT isahl_auth.age_ngac_graph_ready() THEN
        RAISE NOTICE 'AGE 不可用或图未注册，跳过重建';
        RETURN;
    END IF;

    lbl_ua := ag_catalog._label_id('isahl_auth'::name, 'UserAttribute'::name)::integer;
    lbl_oa := ag_catalog._label_id('isahl_auth'::name, 'ObjectAttribute'::name)::integer;
    lbl_user := ag_catalog._label_id('isahl_auth'::name, 'User'::name)::integer;


    DROP TABLE IF EXISTS pg_temp.ua_map;
    DROP TABLE IF EXISTS pg_temp.oa_map;
    DROP TABLE IF EXISTS pg_temp.user_map;

    -- 清空投影（业务 label + 基础表；RESTART IDENTITY 归零各 label 序列）
    TRUNCATE isahl_auth."User", isahl_auth."UserAttribute", isahl_auth."ObjectAttribute",
             isahl_auth."HAS_ATTRIBUTE", isahl_auth."PARENT_OF",
             isahl_auth."ASSOCIATION", isahl_auth."PROHIBITION",
             isahl_auth._ag_label_vertex, isahl_auth._ag_label_edge RESTART IDENTITY;

    -- UA 顶点（rank → graphid 映射）
    CREATE TEMP TABLE ua_map ON COMMIT DROP AS
    SELECT id AS row_id,
           ag_catalog._graphid(lbl_ua,
               row_number() OVER (ORDER BY id)) AS graphid,
           row_number() OVER (ORDER BY id) AS rnk
      FROM isahl_auth.ngac_user_attribute
     WHERE deleted_at IS NULL;

    INSERT INTO isahl_auth."UserAttribute" (id, properties)
    SELECT m.graphid,
           jsonb_build_object('id', ua.id, 'o_name', ua.o_name,
                              'fk_policy_class', ua.fk_policy_class)::text::ag_catalog.agtype
      FROM isahl_auth.ngac_user_attribute ua
      JOIN ua_map m ON m.row_id = ua.id
     WHERE ua.deleted_at IS NULL;

    SELECT COALESCE(max(rnk), 1) INTO max_rnk FROM ua_map;
    PERFORM setval('isahl_auth."UserAttribute_id_seq"'::regclass, max_rnk);

    -- OA 顶点
    CREATE TEMP TABLE oa_map ON COMMIT DROP AS
    SELECT id AS row_id,
           ag_catalog._graphid(lbl_oa,
               row_number() OVER (ORDER BY id)) AS graphid,
           row_number() OVER (ORDER BY id) AS rnk
      FROM isahl_auth.ngac_object_attribute
     WHERE deleted_at IS NULL;

    INSERT INTO isahl_auth."ObjectAttribute" (id, properties)
    SELECT m.graphid,
           jsonb_build_object('id', oa.id, 'o_name', oa.o_name,
                              'resource_type', oa.resource_type,
                              'fk_resource', oa.fk_resource,
                              'resource_identifier', oa.resource_identifier)::text::ag_catalog.agtype
      FROM isahl_auth.ngac_object_attribute oa
      JOIN oa_map m ON m.row_id = oa.id
     WHERE oa.deleted_at IS NULL;

    SELECT COALESCE(max(rnk), 1) INTO max_rnk FROM oa_map;
    PERFORM setval('isahl_auth."ObjectAttribute_id_seq"'::regclass, max_rnk);

    -- User 顶点（出现在任一有效指派的用户）
    CREATE TEMP TABLE user_map ON COMMIT DROP AS
    SELECT fk_user AS row_id,
           ag_catalog._graphid(lbl_user,
               row_number() OVER (ORDER BY fk_user)) AS graphid,
           row_number() OVER (ORDER BY fk_user) AS rnk
      FROM (SELECT DISTINCT fk_user FROM isahl_auth.ngac_user_rr_attribute
             WHERE deleted_at IS NULL AND (expires_at IS NULL OR expires_at > now())) s;

    INSERT INTO isahl_auth."User" (id, properties)
    SELECT m.graphid, jsonb_build_object('id', m.row_id)::text::ag_catalog.agtype
      FROM user_map m;

    SELECT COALESCE(max(rnk), 1) INTO max_rnk FROM user_map;
    PERFORM setval('isahl_auth."User_id_seq"'::regclass, max_rnk);

    -- UA 层级边（PARENT_OF）
    INSERT INTO isahl_auth."PARENT_OF" (start_id, end_id)
    SELECT c.graphid, p.graphid
      FROM isahl_auth.ngac_user_attribute ua
      JOIN LATERAL unnest(ua.ancestor_ids) AS anc(ancestor) ON true
      JOIN ua_map c ON c.row_id = ua.id
      JOIN ua_map p ON p.row_id = anc.ancestor
     WHERE ua.deleted_at IS NULL;

    -- OA 层级边
    INSERT INTO isahl_auth."PARENT_OF" (start_id, end_id)
    SELECT c.graphid, p.graphid
      FROM isahl_auth.ngac_object_attribute oa
      JOIN LATERAL unnest(oa.ancestor_ids) AS anc(ancestor) ON true
      JOIN oa_map c ON c.row_id = oa.id
      JOIN oa_map p ON p.row_id = anc.ancestor
     WHERE oa.deleted_at IS NULL;

    -- 指派边（HAS_ATTRIBUTE；row_id 属性承载指派行 id）
    INSERT INTO isahl_auth."HAS_ATTRIBUTE" (start_id, end_id, properties)
    SELECT u.graphid, ua.graphid,
           jsonb_build_object('row_id', a.id, 'expires_at', a.expires_at)::text::ag_catalog.agtype
      FROM isahl_auth.ngac_user_rr_attribute a
      JOIN user_map u ON u.row_id = a.fk_user
      JOIN ua_map ua ON ua.row_id = a.fk_user_attribute
     WHERE a.deleted_at IS NULL AND (a.expires_at IS NULL OR a.expires_at > now());

    -- Association 规则边
    INSERT INTO isahl_auth."ASSOCIATION" (start_id, end_id, properties)
    SELECT ua.graphid, oa.graphid,
           jsonb_build_object('id', a.id, 'rights', COALESCE(a.ak_access_rights, ARRAY[]::bigint[]),
                              'conditions', COALESCE(a.conditions, '{}'::jsonb))::text::ag_catalog.agtype
      FROM isahl_auth.ngac_association a
      JOIN ua_map ua ON ua.row_id = a.fk_user_attribute
      JOIN oa_map oa ON oa.row_id = a.fk_object_attribute
     WHERE a.deleted_at IS NULL;

    -- Prohibition 规则边
    INSERT INTO isahl_auth."PROHIBITION" (start_id, end_id, properties)
    SELECT ua.graphid, oa.graphid,
           jsonb_build_object('id', p.id, 'rights', COALESCE(p.ak_access_rights, ARRAY[]::bigint[]),
                              'is_active', COALESCE(p.is_active, true),
                              'conditions', COALESCE(p.conditions, '{}'::jsonb))::text::ag_catalog.agtype
      FROM isahl_auth.ngac_prohibition p
      JOIN ua_map ua ON ua.row_id = p.fk_user_attribute
      JOIN oa_map oa ON oa.row_id = p.fk_object_attribute
     WHERE p.deleted_at IS NULL;

    DROP TABLE IF EXISTS pg_temp.ua_map;
    DROP TABLE IF EXISTS pg_temp.oa_map;
    DROP TABLE IF EXISTS pg_temp.user_map;
END $fn$;

-- ═══ 6) 对账检查（表 ↔ 图计数；drift ≠ 0 → 投影滞后，rebuild 自愈）═══
-- plpgsql 而非 LANGUAGE sql：SQL 语言函数建函数即校验关系引用，无 AGE 环境
-- （标签表 UserAttribute/ASSOCIATION 等不存在）会在迁移期直接 ERROR，违反 D9
-- 「AGE 缺失全 no-op」。plpgsql 语句首次执行才规划——守卫早退后标签表查询
-- 永不规划，无 AGE 环境创建与调用均安全。
CREATE OR REPLACE FUNCTION isahl_auth.age_ngac_projection_diff()
RETURNS TABLE(entity text, table_rows bigint, graph_rows bigint, drift bigint)
LANGUAGE plpgsql STABLE
AS $fn$
BEGIN
    IF NOT isahl_auth.age_ngac_graph_ready() THEN
        RETURN QUERY SELECT 'user_attribute'::text, 0::bigint, 0::bigint, 0::bigint;
        RETURN;
    END IF;
    RETURN QUERY
    SELECT v.entity, v.tbl, v.graph, v.tbl - v.graph FROM (
        SELECT 'user_attribute'::text AS entity,
               (SELECT count(*) FROM isahl_auth.ngac_user_attribute WHERE deleted_at IS NULL) AS tbl,
               (SELECT count(*) FROM isahl_auth."UserAttribute") AS graph
        UNION ALL
        SELECT 'object_attribute',
               (SELECT count(*) FROM isahl_auth.ngac_object_attribute WHERE deleted_at IS NULL),
               (SELECT count(*) FROM isahl_auth."ObjectAttribute")
        UNION ALL
        SELECT 'user_assignment',
               (SELECT count(*) FROM isahl_auth.ngac_user_rr_attribute a
                 WHERE a.deleted_at IS NULL AND (a.expires_at IS NULL OR a.expires_at > now())
                   AND EXISTS (SELECT 1 FROM isahl_auth.ngac_user_attribute ua
                                WHERE ua.id = a.fk_user_attribute AND ua.deleted_at IS NULL)),
               (SELECT count(*) FROM isahl_auth."HAS_ATTRIBUTE")
        UNION ALL
        SELECT 'association',
               (SELECT count(*) FROM isahl_auth.ngac_association WHERE deleted_at IS NULL),
               (SELECT count(*) FROM isahl_auth."ASSOCIATION")
        UNION ALL
        SELECT 'prohibition',
               (SELECT count(*) FROM isahl_auth.ngac_prohibition WHERE deleted_at IS NULL),
               (SELECT count(*) FROM isahl_auth."PROHIBITION")
    ) v;
END
$fn$;

-- 首次落地即重建一次（幂等；空库时为空跑）
SELECT isahl_auth.age_rebuild_ngac_graph();
