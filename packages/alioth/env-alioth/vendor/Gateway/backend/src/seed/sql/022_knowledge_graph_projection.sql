-- 022_knowledge_graph_projection.sql
-- 知识图谱投影（change extend-knowledge-graph-cypher-backend B-0，design D1/D2）
--
-- 纯重建制（零 isahl 触发器——isahl 结构冻结 + 业务写路径零开销）；幂等重放安全；
-- AGE 缺失环境整体 no-op。模式复刻 A 段 037（add-age-graph-projection）：
-- 手工图引导 + 纯 SQL label + O(n) rank 直插（zuid 行 id 超 graphid 2^48 位宽）+ 对账。
-- 白名单 = AVIC DOMAIN_TABLES 全域表（FROM ONLY 层表互斥）+ 三桥 + contract REL。
--
-- AGE 函数名（2026-09-20 修正）：对账函数体用 `ag_catalog.agtype_to_json`（AGE 1.7.0 提供，
-- 返回 json）。原文使用的 `agtype_to_jsonb` **在任何 AGE 版本都不存在**——SQL 语言函数体在
-- 创建时即校验（check_function_bodies），该 CREATE FUNCTION 一直失败；而迁移重放当时以
-- `psql -f`（无 ON_ERROR_STOP）+ `2>/dev/null` 执行，失败被静默吞掉 ⇒ 对账函数从未落地
-- （实测同一报错在 PG 日志累计 728 次）。重放层已改为 fail-loud（见 change
-- fix-age-graph-orphan-on-rebuild），该误名同步修正。

-- ═══ 0) schema 前置（守卫函数承载面）═══
CREATE SCHEMA IF NOT EXISTS isahl_knowledge;

-- ═══ 1) 图可用性守卫 ═══
CREATE OR REPLACE FUNCTION isahl_knowledge.age_graph_ready()
RETURNS boolean LANGUAGE plpgsql STABLE AS $fn$
BEGIN
    RETURN EXISTS(SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'age')
       AND EXISTS(SELECT 1 FROM ag_catalog.ag_graph WHERE name = 'isahl_knowledge');
EXCEPTION WHEN OTHERS THEN
    RETURN false;
END $fn$;

-- ═══ 2) 图引导（幂等）：schema + label id 序列 + ag_graph 注册 ═══
DO $bootstrap$
DECLARE
    orphan_row_cnt bigint;
BEGIN
    IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'age') THEN
        RAISE NOTICE 'age 扩展未安装，跳过知识图引导';
        RETURN;
    END IF;
    IF EXISTS(SELECT 1 FROM ag_catalog.ag_graph WHERE name = 'isahl_knowledge') THEN
        RETURN;
    END IF;
    CREATE SCHEMA IF NOT EXISTS isahl_knowledge;
    -- 残留处理（2026-09-20 修正，restore 自愈实测）：本图是纯重建制派生投影（关系表为权威源），
    -- 残留一律「先解锁 → 重建覆盖」，MUST NOT 抛错阻断自愈——`count(*)` 经继承计入子表
    -- （`Node` 等正常图数据）会把 restore 态误判为危险残留（实测 104 行）而中止整批迁移；
    -- `DROP TABLE` 亦须 `CASCADE`（子表存在时无 CASCADE 报 cannot drop … because other objects depend on it）。
    IF to_regclass('isahl_knowledge._ag_label_vertex') IS NOT NULL
       OR to_regclass('isahl_knowledge._ag_label_edge') IS NOT NULL THEN
        SELECT COALESCE((SELECT count(*) FROM isahl_knowledge._ag_label_vertex), 0) INTO orphan_row_cnt;
        IF orphan_row_cnt > 0 THEN
            RAISE NOTICE 'isahl_knowledge：检测到既有投影数据 % 行（注册缺失或漂移）——按幂等重建覆盖', orphan_row_cnt;
        END IF;
        -- CASCADE：业务 label 表（Node/PARENT/REL/三桥）INHERITS 基础表，无 CASCADE 时
        -- DROP 被继承依赖阻塞（实测：`无法删除 表 _ag_label_vertex 因为有其它对象倚赖它`）
        -- → restore/同步后带子表的残留状态自愈必失败。空值守卫在上（含子表行，
        -- 继承扫描不带 ONLY），CASCADE 只会删除空残留，且全部对象由后续 label ensure 重建。
        DROP TABLE IF EXISTS isahl_knowledge._ag_label_vertex CASCADE;
        DROP TABLE IF EXISTS isahl_knowledge._ag_label_edge CASCADE;
        DROP SEQUENCE IF EXISTS isahl_knowledge._ag_label_vertex_id_seq;
        DROP SEQUENCE IF EXISTS isahl_knowledge._ag_label_edge_id_seq;
        DROP SEQUENCE IF EXISTS isahl_knowledge._label_id_seq;
    END IF;
    CREATE SEQUENCE isahl_knowledge._label_id_seq AS integer MAXVALUE 65535 CYCLE;
    INSERT INTO ag_catalog.ag_graph (graphid, name, namespace)
    VALUES ('isahl_knowledge'::regnamespace::oid, 'isahl_knowledge', 'isahl_knowledge'::regnamespace);
END
$bootstrap$;

-- ═══ 3) Label 全集（纯 SQL 手工；形态对齐官方 pg_dump 实证）═══
DO $labels$
DECLARE
    lname text;
    goid oid := 'isahl_knowledge'::regnamespace::oid;
BEGIN
    IF NOT EXISTS(SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'age') THEN
        RETURN;
    END IF;
    IF NOT EXISTS(SELECT 1 FROM ag_catalog.ag_graph WHERE name = 'isahl_knowledge') THEN
        RAISE NOTICE '图未注册，跳过 label 创建';
        RETURN;
    END IF;

    -- 基础 vertex label
    CREATE TABLE IF NOT EXISTS isahl_knowledge._ag_label_vertex (
        id ag_catalog.graphid NOT NULL,
        properties ag_catalog.agtype DEFAULT ag_catalog.agtype_build_map() NOT NULL
    );
    CREATE SEQUENCE IF NOT EXISTS isahl_knowledge._ag_label_vertex_id_seq
        START WITH 1 INCREMENT BY 1 NO MINVALUE MAXVALUE 281474976710655 CACHE 1;
    ALTER SEQUENCE isahl_knowledge._ag_label_vertex_id_seq OWNED BY isahl_knowledge._ag_label_vertex.id;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint c JOIN pg_class t ON t.oid = c.conrelid
                    JOIN pg_namespace n ON n.oid = t.relnamespace
                   WHERE n.nspname = 'isahl_knowledge' AND t.relname = '_ag_label_vertex'
                     AND c.conname = '_ag_label_vertex_pkey') THEN
        ALTER TABLE isahl_knowledge._ag_label_vertex ADD CONSTRAINT _ag_label_vertex_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM ag_catalog.ag_label WHERE graph = goid AND name = '_ag_label_vertex') THEN
        INSERT INTO ag_catalog.ag_label (name, graph, id, kind, relation, seq_name)
        VALUES ('_ag_label_vertex', goid, nextval('isahl_knowledge._label_id_seq'), 'v'::ag_catalog.label_kind,
                'isahl_knowledge._ag_label_vertex'::regclass, '_ag_label_vertex_id_seq');
    END IF;
    ALTER TABLE isahl_knowledge._ag_label_vertex ALTER COLUMN id SET DEFAULT
        ag_catalog._graphid((ag_catalog._label_id('isahl_knowledge'::name, '_ag_label_vertex'::name))::integer,
                             nextval('isahl_knowledge._ag_label_vertex_id_seq'::regclass));

    -- 基础 edge label
    CREATE TABLE IF NOT EXISTS isahl_knowledge._ag_label_edge (
        id ag_catalog.graphid NOT NULL,
        start_id ag_catalog.graphid NOT NULL,
        end_id ag_catalog.graphid NOT NULL,
        properties ag_catalog.agtype DEFAULT ag_catalog.agtype_build_map() NOT NULL
    );
    CREATE INDEX IF NOT EXISTS _ag_label_edge_start_id_idx
        ON isahl_knowledge._ag_label_edge USING btree (start_id ag_catalog.graphid_ops);
    CREATE INDEX IF NOT EXISTS _ag_label_edge_end_id_idx
        ON isahl_knowledge._ag_label_edge USING btree (end_id ag_catalog.graphid_ops);
    CREATE SEQUENCE IF NOT EXISTS isahl_knowledge._ag_label_edge_id_seq
        START WITH 1 INCREMENT BY 1 NO MINVALUE MAXVALUE 281474976710655 CACHE 1;
    ALTER SEQUENCE isahl_knowledge._ag_label_edge_id_seq OWNED BY isahl_knowledge._ag_label_edge.id;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint c JOIN pg_class t ON t.oid = c.conrelid
                    JOIN pg_namespace n ON n.oid = t.relnamespace
                   WHERE n.nspname = 'isahl_knowledge' AND t.relname = '_ag_label_edge'
                     AND c.conname = '_ag_label_edge_pkey') THEN
        ALTER TABLE isahl_knowledge._ag_label_edge ADD CONSTRAINT _ag_label_edge_pkey PRIMARY KEY (id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM ag_catalog.ag_label WHERE graph = goid AND name = '_ag_label_edge') THEN
        INSERT INTO ag_catalog.ag_label (name, graph, id, kind, relation, seq_name)
        VALUES ('_ag_label_edge', goid, nextval('isahl_knowledge._label_id_seq'), 'e'::ag_catalog.label_kind,
                'isahl_knowledge._ag_label_edge'::regclass, '_ag_label_edge_id_seq');
    END IF;
    ALTER TABLE isahl_knowledge._ag_label_edge ALTER COLUMN id SET DEFAULT
        ag_catalog._graphid((ag_catalog._label_id('isahl_knowledge'::name, '_ag_label_edge'::name))::integer,
                             nextval('isahl_knowledge._ag_label_edge_id_seq'::regclass));

    -- 业务 labels：Node（顶点）+ 六边
    FOREACH lname IN ARRAY ARRAY['Node'] LOOP
        EXECUTE format('CREATE TABLE IF NOT EXISTS isahl_knowledge.%I () INHERITS (isahl_knowledge._ag_label_vertex)', lname);
        EXECUTE format('CREATE SEQUENCE IF NOT EXISTS isahl_knowledge.%I START WITH 1 INCREMENT BY 1 NO MINVALUE MAXVALUE 281474976710655 CACHE 1', lname || '_id_seq');
        EXECUTE format('ALTER SEQUENCE isahl_knowledge.%I OWNED BY isahl_knowledge.%I.id', lname || '_id_seq', lname);
        IF NOT EXISTS (SELECT 1 FROM pg_constraint c JOIN pg_class t ON t.oid = c.conrelid
                        JOIN pg_namespace n ON n.oid = t.relnamespace
                       WHERE n.nspname = 'isahl_knowledge' AND t.relname = lname
                         AND c.conname = lname || '_pkey') THEN
            EXECUTE format('ALTER TABLE isahl_knowledge.%I ADD CONSTRAINT %I PRIMARY KEY (id)', lname, lname || '_pkey');
        END IF;
        IF NOT EXISTS (SELECT 1 FROM ag_catalog.ag_label WHERE graph = goid AND name = lname) THEN
            INSERT INTO ag_catalog.ag_label (name, graph, id, kind, relation, seq_name)
            VALUES (lname, goid, nextval('isahl_knowledge._label_id_seq'), 'v'::ag_catalog.label_kind,
                    format('isahl_knowledge.%I', lname)::regclass, lname || '_id_seq');
        END IF;
        EXECUTE format('ALTER TABLE isahl_knowledge.%I ALTER COLUMN id SET DEFAULT ag_catalog._graphid((ag_catalog._label_id(''isahl_knowledge''::name, %L::name))::integer, nextval(%L::regclass))',
                        lname, lname, format('isahl_knowledge.%I', lname || '_id_seq'));
    END LOOP;
    FOREACH lname IN ARRAY ARRAY['PARENT', 'PREVIOUS', 'BRIDGE_LAW', 'BRIDGE_REFERENCE', 'BRIDGE_FORMULA', 'REL'] LOOP
        EXECUTE format('CREATE TABLE IF NOT EXISTS isahl_knowledge.%I () INHERITS (isahl_knowledge._ag_label_edge)', lname);
        EXECUTE format('CREATE SEQUENCE IF NOT EXISTS isahl_knowledge.%I START WITH 1 INCREMENT BY 1 NO MINVALUE MAXVALUE 281474976710655 CACHE 1', lname || '_id_seq');
        EXECUTE format('ALTER SEQUENCE isahl_knowledge.%I OWNED BY isahl_knowledge.%I.id', lname || '_id_seq', lname);
        IF NOT EXISTS (SELECT 1 FROM pg_constraint c JOIN pg_class t ON t.oid = c.conrelid
                        JOIN pg_namespace n ON n.oid = t.relnamespace
                       WHERE n.nspname = 'isahl_knowledge' AND t.relname = lname
                         AND c.conname = lname || '_pkey') THEN
            EXECUTE format('ALTER TABLE isahl_knowledge.%I ADD CONSTRAINT %I PRIMARY KEY (id)', lname, lname || '_pkey');
        END IF;
        IF NOT EXISTS (SELECT 1 FROM ag_catalog.ag_label WHERE graph = goid AND name = lname) THEN
            INSERT INTO ag_catalog.ag_label (name, graph, id, kind, relation, seq_name)
            VALUES (lname, goid, nextval('isahl_knowledge._label_id_seq'), 'e'::ag_catalog.label_kind,
                    format('isahl_knowledge.%I', lname)::regclass, lname || '_id_seq');
        END IF;
        EXECUTE format('ALTER TABLE isahl_knowledge.%I ALTER COLUMN id SET DEFAULT ag_catalog._graphid((ag_catalog._label_id(''isahl_knowledge''::name, %L::name))::integer, nextval(%L::regclass))',
                        lname, lname, format('isahl_knowledge.%I', lname || '_id_seq'));
    END LOOP;
END
$labels$;

-- ═══ 4) 全量重建（O(n) rank 直插；幂等）═══
CREATE OR REPLACE FUNCTION isahl_knowledge.age_rebuild_knowledge_graph()
RETURNS void LANGUAGE plpgsql
AS $fn$
DECLARE
    -- 标签 id 延迟到守卫之后取（无 AGE 环境函数入口即解析 ag_catalog._label_id
    -- 会撞 schema does not exist；plpgsql 表达式首次执行才规划，守卫早退后永不触达）
    lbl_node integer;
    lbl_parent integer;
    lbl_prev integer;
    lbl_bl integer;
    lbl_br integer;
    lbl_bf integer;
    lbl_rel integer;
    max_rnk bigint;
BEGIN
    IF NOT isahl_knowledge.age_graph_ready() THEN
        RAISE NOTICE 'AGE 不可用或知识图未注册，跳过重建';
        RETURN;
    END IF;

    lbl_node := ag_catalog._label_id('isahl_knowledge'::name, 'Node'::name)::integer;
    lbl_parent := ag_catalog._label_id('isahl_knowledge'::name, 'PARENT'::name)::integer;
    lbl_prev := ag_catalog._label_id('isahl_knowledge'::name, 'PREVIOUS'::name)::integer;
    lbl_bl := ag_catalog._label_id('isahl_knowledge'::name, 'BRIDGE_LAW'::name)::integer;
    lbl_br := ag_catalog._label_id('isahl_knowledge'::name, 'BRIDGE_REFERENCE'::name)::integer;
    lbl_bf := ag_catalog._label_id('isahl_knowledge'::name, 'BRIDGE_FORMULA'::name)::integer;
    lbl_rel := ag_catalog._label_id('isahl_knowledge'::name, 'REL'::name)::integer;

    DROP TABLE IF EXISTS pg_temp.k_node_map;
    TRUNCATE isahl_knowledge."Node", isahl_knowledge."PARENT", isahl_knowledge."PREVIOUS",
             isahl_knowledge."BRIDGE_LAW", isahl_knowledge."BRIDGE_REFERENCE",
             isahl_knowledge."BRIDGE_FORMULA", isahl_knowledge."REL",
             isahl_knowledge._ag_label_vertex, isahl_knowledge._ag_label_edge RESTART IDENTITY;

    -- 顶点（白名单 23 表 FROM ONLY + contract 起点；全局 rank → graphid）
    CREATE TEMP TABLE k_node_map ON COMMIT DROP AS
    WITH src AS (
  SELECT t.id, 'zc_id_stan-air-caac'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-caac" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-air-caac-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-caac-article" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-air-faa'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-faa" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-air-faa-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-faa-article" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-air-easa'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-easa" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-air-easa-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-easa-article" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-air-icao'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-icao" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-air-icao-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-icao-article" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-fin-cas'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-fin-cas" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-fin-cas-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-fin-cas-article" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-fin-ifrs'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-fin-ifrs" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_stan-fin-ifrs-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-fin-ifrs-article" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-civil-code'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-civil-code" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-civil-book'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-civil-book" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-civil-chapter'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-civil-chapter" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-civil-section'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-civil-section" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-civil-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-civil-article" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-common-statute'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-statute" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-common-title'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-title" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-common-chapter'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-chapter" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-common-section'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-section" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-common-case'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-case" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT t.id, 'zc_id_law-common-holding'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-holding" t WHERE t.deleted_at IS NULL
      UNION ALL
  SELECT DISTINCT r.ref_left AS id, 'zc_id_contract'::text AS origin, NULL::text AS code, NULL::text AS name FROM isahl."zc_id_contract_rr_law" r WHERE r.deleted_at IS NULL
    )
    SELECT origin, id,
           ag_catalog._graphid(lbl_node, row_number() OVER (ORDER BY origin, id)) AS graphid,
           row_number() OVER (ORDER BY origin, id) AS rnk
      FROM src;

    INSERT INTO isahl_knowledge."Node" (id, properties)
    SELECT m.graphid,
           jsonb_build_object('id', m.id, 'origin', m.origin,
                              'code', s.code, 'name', s.name)::text::ag_catalog.agtype
      FROM k_node_map m
      JOIN (
  SELECT t.id, 'zc_id_stan-air-caac'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-caac" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-air-caac-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-caac-article" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-air-faa'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-faa" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-air-faa-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-faa-article" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-air-easa'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-easa" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-air-easa-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-easa-article" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-air-icao'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-icao" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-air-icao-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-air-icao-article" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-fin-cas'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-fin-cas" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-fin-cas-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-fin-cas-article" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-fin-ifrs'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-fin-ifrs" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_stan-fin-ifrs-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_stan-fin-ifrs-article" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-civil-code'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-civil-code" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-civil-book'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-civil-book" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-civil-chapter'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-civil-chapter" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-civil-section'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-civil-section" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-civil-article'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-civil-article" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-common-statute'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-statute" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-common-title'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-title" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-common-chapter'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-chapter" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-common-section'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-section" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-common-case'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-case" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT t.id, 'zc_id_law-common-holding'::text AS origin, t.code, t.notice AS name FROM ONLY isahl."zc_id_law-common-holding" t WHERE t.deleted_at IS NULL
        UNION ALL
  SELECT DISTINCT r.ref_left AS id, 'zc_id_contract'::text AS origin, NULL::text AS code, NULL::text AS name FROM isahl."zc_id_contract_rr_law" r WHERE r.deleted_at IS NULL
      ) s ON s.origin = m.origin AND s.id = m.id;

    SELECT COALESCE(max(rnk), 1) INTO max_rnk FROM k_node_map;
    PERFORM setval('isahl_knowledge."Node_id_seq"'::regclass, max_rnk);

    -- PARENT 边（fk_parent；端点同 origin 或跨 origin 均经 map 解析）
    INSERT INTO isahl_knowledge."PARENT" (start_id, end_id)
    SELECT c.graphid, p.graphid
      FROM (
  SELECT 'zc_id_stan-air-caac'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-air-caac" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-caac-article'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-air-caac-article" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-faa'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-air-faa" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-faa-article'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-air-faa-article" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-easa'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-air-easa" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-easa-article'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-air-easa-article" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-icao'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-air-icao" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-icao-article'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-air-icao-article" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-fin-cas'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-fin-cas" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-fin-cas-article'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-fin-cas-article" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-fin-ifrs'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-fin-ifrs" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-fin-ifrs-article'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_stan-fin-ifrs-article" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-civil-code'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-civil-code" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-civil-book'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-civil-book" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-civil-chapter'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-civil-chapter" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-civil-section'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-civil-section" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-civil-article'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-civil-article" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-statute'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-common-statute" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-title'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-common-title" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-chapter'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-common-chapter" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-section'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-common-section" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-case'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-common-case" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-holding'::text AS origin, t.id, t.fk_parent AS other FROM ONLY isahl."zc_id_law-common-holding" t WHERE t.deleted_at IS NULL AND t.fk_parent IS NOT NULL
      ) e
      JOIN k_node_map c ON c.origin = e.origin AND c.id = e.id
      JOIN k_node_map p ON p.id = e.other;

    -- PREVIOUS 边（fk_previous 版本链）
    INSERT INTO isahl_knowledge."PREVIOUS" (start_id, end_id)
    SELECT c.graphid, p.graphid
      FROM (
  SELECT 'zc_id_stan-air-caac'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-air-caac" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-caac-article'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-air-caac-article" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-faa'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-air-faa" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-faa-article'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-air-faa-article" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-easa'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-air-easa" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-easa-article'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-air-easa-article" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-icao'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-air-icao" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-air-icao-article'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-air-icao-article" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-fin-cas'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-fin-cas" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-fin-cas-article'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-fin-cas-article" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-fin-ifrs'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-fin-ifrs" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_stan-fin-ifrs-article'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_stan-fin-ifrs-article" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-civil-code'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-civil-code" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-civil-book'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-civil-book" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-civil-chapter'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-civil-chapter" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-civil-section'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-civil-section" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-civil-article'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-civil-article" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-statute'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-common-statute" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-title'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-common-title" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-chapter'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-common-chapter" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-section'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-common-section" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-case'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-common-case" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
        UNION ALL
  SELECT 'zc_id_law-common-holding'::text AS origin, t.id, t.fk_previous AS other FROM ONLY isahl."zc_id_law-common-holding" t WHERE t.deleted_at IS NULL AND t.fk_previous IS NOT NULL
      ) e
      JOIN k_node_map c ON c.origin = e.origin AND c.id = e.id
      JOIN k_node_map p ON p.origin = e.origin AND p.id = e.other;

    -- 三桥边（两端均在顶点集才投影）
    INSERT INTO isahl_knowledge."BRIDGE_LAW" (start_id, end_id)
    SELECT l.graphid, r.graphid FROM (  SELECT r.ref_left AS l, r.ref_right AS rr FROM isahl."zc_id_standard_rr_law" r WHERE r.deleted_at IS NULL) b
      JOIN k_node_map l ON l.id = b.l JOIN k_node_map r ON r.id = b.rr;
    INSERT INTO isahl_knowledge."BRIDGE_REFERENCE" (start_id, end_id)
    SELECT l.graphid, r.graphid FROM (  SELECT r.ref_left AS l, r.ref_right AS rr FROM isahl."zc_id_standard_rr_reference" r WHERE r.deleted_at IS NULL) b
      JOIN k_node_map l ON l.id = b.l JOIN k_node_map r ON r.id = b.rr;
    INSERT INTO isahl_knowledge."BRIDGE_FORMULA" (start_id, end_id)
    SELECT l.graphid, r.graphid FROM (  SELECT r.ref_left AS l, r.ref_right AS rr FROM isahl."zc_id_standard_r_formula" r WHERE r.deleted_at IS NULL) b
      JOIN k_node_map l ON l.id = b.l JOIN k_node_map r ON r.id = b.rr;

    -- REL 边（contract_rr_law）
    INSERT INTO isahl_knowledge."REL" (start_id, end_id)
    SELECT l.graphid, r.graphid FROM (  SELECT r.ref_left AS l, r.ref_right AS rr FROM isahl."zc_id_contract_rr_law" r WHERE r.deleted_at IS NULL) b
      JOIN k_node_map l ON l.origin = 'zc_id_contract' AND l.id = b.l
      JOIN k_node_map r ON r.id = b.rr;

    DROP TABLE IF EXISTS pg_temp.k_node_map;
END $fn$;

-- ═══ 5) 对账（表 ↔ 图；桥/REL 口径 = 两端均在顶点集的行数）═══
CREATE OR REPLACE FUNCTION isahl_knowledge.age_knowledge_projection_diff()
RETURNS TABLE(entity text, table_rows bigint, graph_rows bigint, drift bigint)
-- plpgsql 而非 LANGUAGE sql：SQL 语言函数建函数即校验关系引用，无 AGE 环境
-- （isahl_knowledge."Node"/"REL"/桥标签表不存在）迁移期必撞 relation does not
-- exist。plpgsql 语句首次执行才规划——守卫早退后图查询永不规划。
LANGUAGE plpgsql STABLE AS $fn$
BEGIN
    IF NOT isahl_knowledge.age_graph_ready() THEN
        RETURN;
    END IF;
    RETURN QUERY
    WITH nmap AS (
        SELECT ag_catalog.agtype_to_json(properties)->>'origin' AS origin, count(*) AS graph
          FROM isahl_knowledge."Node" GROUP BY 1
    ), ready AS (
        SELECT isahl_knowledge.age_graph_ready() AS ok
    )
    SELECT v.entity, v.tbl, COALESCE(n.graph, 0), v.tbl - COALESCE(n.graph, 0)
      FROM (
    SELECT 'zc_id_stan-air-caac' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-caac" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-caac') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-caac-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-caac-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-caac-article') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-faa' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-faa" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-faa') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-faa-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-faa-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-faa-article') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-easa' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-easa" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-easa') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-easa-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-easa-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-easa-article') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-icao' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-icao" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-icao') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-icao-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-icao-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-icao-article') AS graph
    UNION ALL
    SELECT 'zc_id_stan-fin-cas' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-fin-cas" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-fin-cas') AS graph
    UNION ALL
    SELECT 'zc_id_stan-fin-cas-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-fin-cas-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-fin-cas-article') AS graph
    UNION ALL
    SELECT 'zc_id_stan-fin-ifrs' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-fin-ifrs" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-fin-ifrs') AS graph
    UNION ALL
    SELECT 'zc_id_stan-fin-ifrs-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-fin-ifrs-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-fin-ifrs-article') AS graph
    UNION ALL
    SELECT 'zc_id_law-civil-code' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-civil-code" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-civil-code') AS graph
    UNION ALL
    SELECT 'zc_id_law-civil-book' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-civil-book" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-civil-book') AS graph
    UNION ALL
    SELECT 'zc_id_law-civil-chapter' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-civil-chapter" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-civil-chapter') AS graph
    UNION ALL
    SELECT 'zc_id_law-civil-section' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-civil-section" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-civil-section') AS graph
    UNION ALL
    SELECT 'zc_id_law-civil-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-civil-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-civil-article') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-statute' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-statute" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-statute') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-title' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-title" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-title') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-chapter' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-chapter" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-chapter') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-section' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-section" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-section') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-case' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-case" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-case') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-holding' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-holding" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-holding') AS graph
    UNION ALL
    SELECT 'zc_id_contract' AS entity,
           (SELECT count(DISTINCT r.ref_left) FROM isahl."zc_id_contract_rr_law" r WHERE r.deleted_at IS NULL) AS tbl,
           0
    ) v
    LEFT JOIN nmap n ON n.origin = v.entity, ready
    WHERE NOT ready.ok AND false
    UNION ALL
    SELECT v.entity, v.tbl, COALESCE(n.graph, 0), v.tbl - COALESCE(n.graph, 0)
      FROM (
    SELECT 'zc_id_stan-air-caac' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-caac" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-caac') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-caac-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-caac-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-caac-article') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-faa' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-faa" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-faa') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-faa-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-faa-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-faa-article') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-easa' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-easa" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-easa') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-easa-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-easa-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-easa-article') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-icao' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-icao" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-icao') AS graph
    UNION ALL
    SELECT 'zc_id_stan-air-icao-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-air-icao-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-air-icao-article') AS graph
    UNION ALL
    SELECT 'zc_id_stan-fin-cas' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-fin-cas" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-fin-cas') AS graph
    UNION ALL
    SELECT 'zc_id_stan-fin-cas-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-fin-cas-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-fin-cas-article') AS graph
    UNION ALL
    SELECT 'zc_id_stan-fin-ifrs' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-fin-ifrs" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-fin-ifrs') AS graph
    UNION ALL
    SELECT 'zc_id_stan-fin-ifrs-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_stan-fin-ifrs-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_stan-fin-ifrs-article') AS graph
    UNION ALL
    SELECT 'zc_id_law-civil-code' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-civil-code" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-civil-code') AS graph
    UNION ALL
    SELECT 'zc_id_law-civil-book' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-civil-book" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-civil-book') AS graph
    UNION ALL
    SELECT 'zc_id_law-civil-chapter' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-civil-chapter" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-civil-chapter') AS graph
    UNION ALL
    SELECT 'zc_id_law-civil-section' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-civil-section" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-civil-section') AS graph
    UNION ALL
    SELECT 'zc_id_law-civil-article' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-civil-article" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-civil-article') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-statute' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-statute" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-statute') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-title' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-title" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-title') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-chapter' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-chapter" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-chapter') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-section' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-section" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-section') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-case' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-case" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-case') AS graph
    UNION ALL
    SELECT 'zc_id_law-common-holding' AS entity,
           (SELECT count(*) FROM ONLY isahl."zc_id_law-common-holding" WHERE deleted_at IS NULL) AS tbl,
           (SELECT count(*) FROM isahl_knowledge."Node" WHERE ag_catalog.agtype_to_json(properties)->>'origin' = 'zc_id_law-common-holding') AS graph

    ) v
    LEFT JOIN nmap n ON n.origin = v.entity, ready
    WHERE ready.ok
    UNION ALL
    SELECT 'rel_contract_law',
           (SELECT count(*) FROM isahl."zc_id_contract_rr_law" b
             WHERE b.deleted_at IS NULL AND EXISTS (SELECT 1 FROM isahl_knowledge."Node" n
               WHERE ag_catalog.agtype_to_json(n.properties)::jsonb->>'id' = b.ref_left::text)
               AND EXISTS (SELECT 1 FROM isahl_knowledge."Node" n2
               WHERE ag_catalog.agtype_to_json(n2.properties)::jsonb->>'id' = b.ref_right::text)),
           (SELECT count(*) FROM isahl_knowledge."REL")::bigint,
           (SELECT count(*) FROM isahl."zc_id_contract_rr_law" b
             WHERE b.deleted_at IS NULL AND EXISTS (SELECT 1 FROM isahl_knowledge."Node" n
               WHERE ag_catalog.agtype_to_json(n.properties)::jsonb->>'id' = b.ref_left::text)
               AND EXISTS (SELECT 1 FROM isahl_knowledge."Node" n2
               WHERE ag_catalog.agtype_to_json(n2.properties)::jsonb->>'id' = b.ref_right::text))
         - (SELECT count(*) FROM isahl_knowledge."REL")
    UNION ALL
    SELECT 'bridge_law',
           (SELECT count(*) FROM isahl."zc_id_standard_rr_law" b WHERE b.deleted_at IS NULL
             AND EXISTS (SELECT 1 FROM isahl_knowledge."Node" n
               WHERE ag_catalog.agtype_to_json(n.properties)::jsonb->>'id' = b.ref_left::text)
             AND EXISTS (SELECT 1 FROM isahl_knowledge."Node" n2
               WHERE ag_catalog.agtype_to_json(n2.properties)::jsonb->>'id' = b.ref_right::text)),
           (SELECT count(*) FROM isahl_knowledge."BRIDGE_LAW")::bigint,
           (SELECT count(*) FROM isahl."zc_id_standard_rr_law" b WHERE b.deleted_at IS NULL
             AND EXISTS (SELECT 1 FROM isahl_knowledge."Node" n
               WHERE ag_catalog.agtype_to_json(n.properties)::jsonb->>'id' = b.ref_left::text)
             AND EXISTS (SELECT 1 FROM isahl_knowledge."Node" n2
               WHERE ag_catalog.agtype_to_json(n2.properties)::jsonb->>'id' = b.ref_right::text))
         - (SELECT count(*) FROM isahl_knowledge."BRIDGE_LAW");
END
$fn$;

-- ═══ 8) 多跳读投影函数（Rust 侧参数化 Cypher 的唯一可行通道）═══
-- 平台事实（2026-09-20 实测，AGE 1.8.0 + PG18.6 + sqlx 0.9）：
--   · sqlx 的 Bind 一律以 binary 格式发送参数，而 agtype 只接受 AGE 私有二进制布局
--     （绑定 agtype 文本字节报 `agtype_recv: unsupported agtype version number`）；
--   · AGE 拒绝 cast 表达式作第三参（`third argument of cypher function must be a
--     parameter`），且无 text→agtype cast（`cypher(unknown, unknown, text)` 不存在）。
-- 故运行时值 MUST 在库内经 plpgsql 变量 cast 为 agtype 后作第三参传入（变量引用 =
-- Param 节点，AGE 唯一接受形态）；Cypher 文本 MUST 为函数体内字面常量（零拼接、
-- 零动态 SQL）。返回单列 jsonb（Cypher `RETURN` 为单个 map——多列 RETURN 与单列
-- coldeflist 冲突：`return row and column definition list do not match`）。
-- depth 由调用方校验 ∈ [1,3]（1 走单跳 SQL），故此处仅 2/3 两条常量分支。
-- AGE 1.8 无 `|` 边类型联合 VLE（实测 `syntax error 在 "|" 或附近`）→ 四边类型按
-- VLE 分支 UNION；因此混合类型链（如 REL→BRIDGE_LAW）仅 SQL 递归路径可达
-- （守门口径 = Cypher 命中 ⊆ SQL 递归命中，MUST NOT 宣称集合全等）。
-- 无 AGE 环境跳过创建（037 D9 同语义：缺环不阻断）——plpgsql 校验期即解析
-- DECLARE 的 ag_catalog.agtype 类型，缺扩展建函数必撞 schema does not exist；
-- DO+EXECUTE 包装使 CREATE 仅在 AGE 在场时执行（运行时多跳读在无 AGE 环境无消费方）。
DO $wrap$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'age') THEN
        RAISE NOTICE 'age 扩展未安装，跳过 knowledge_multi_hop 创建';
        RETURN;
    END IF;
    EXECUTE $wrapfn$
CREATE OR REPLACE FUNCTION isahl_knowledge.knowledge_multi_hop(
    p_origin text,
    p_id bigint,
    p_depth integer
)
RETURNS SETOF jsonb
LANGUAGE plpgsql
AS $fn$
DECLARE
    m ag_catalog.agtype;
BEGIN
    PERFORM set_config('search_path', 'ag_catalog, pg_temp', true);
    IF p_depth < 2 OR p_depth > 3 THEN
        RAISE EXCEPTION 'knowledge_multi_hop depth 越界（期望 2..3）: %', p_depth;
    END IF;
    m := (jsonb_build_object('origin', p_origin, 'id', p_id)::text)::ag_catalog.agtype;

    IF p_depth = 2 THEN
        RETURN QUERY SELECT to_jsonb(r) FROM ag_catalog.cypher('isahl_knowledge', $cy$
            MATCH p=(s:Node {origin: $origin, id: $id})-[:REL*1..2]->(t:Node)
            RETURN {id: t.id, origin: t.origin, code: t.code, name: t.name,
                    via: [n IN nodes(p) | n.origin]}
            UNION
            MATCH p=(s:Node {origin: $origin, id: $id})-[:BRIDGE_LAW*1..2]->(t:Node)
            RETURN {id: t.id, origin: t.origin, code: t.code, name: t.name,
                    via: [n IN nodes(p) | n.origin]}
            UNION
            MATCH p=(s:Node {origin: $origin, id: $id})-[:BRIDGE_REFERENCE*1..2]->(t:Node)
            RETURN {id: t.id, origin: t.origin, code: t.code, name: t.name,
                    via: [n IN nodes(p) | n.origin]}
            UNION
            MATCH p=(s:Node {origin: $origin, id: $id})-[:BRIDGE_FORMULA*1..2]->(t:Node)
            RETURN {id: t.id, origin: t.origin, code: t.code, name: t.name,
                    via: [n IN nodes(p) | n.origin]}
        $cy$, m) AS (r ag_catalog.agtype);
    ELSE
        RETURN QUERY SELECT to_jsonb(r) FROM ag_catalog.cypher('isahl_knowledge', $cy$
            MATCH p=(s:Node {origin: $origin, id: $id})-[:REL*1..3]->(t:Node)
            RETURN {id: t.id, origin: t.origin, code: t.code, name: t.name,
                    via: [n IN nodes(p) | n.origin]}
            UNION
            MATCH p=(s:Node {origin: $origin, id: $id})-[:BRIDGE_LAW*1..3]->(t:Node)
            RETURN {id: t.id, origin: t.origin, code: t.code, name: t.name,
                    via: [n IN nodes(p) | n.origin]}
            UNION
            MATCH p=(s:Node {origin: $origin, id: $id})-[:BRIDGE_REFERENCE*1..3]->(t:Node)
            RETURN {id: t.id, origin: t.origin, code: t.code, name: t.name,
                    via: [n IN nodes(p) | n.origin]}
            UNION
            MATCH p=(s:Node {origin: $origin, id: $id})-[:BRIDGE_FORMULA*1..3]->(t:Node)
            RETURN {id: t.id, origin: t.origin, code: t.code, name: t.name,
                    via: [n IN nodes(p) | n.origin]}
        $cy$, m) AS (r ag_catalog.agtype);
    END IF;
END $fn$;
    $wrapfn$;
END
$wrap$;


-- 首次落地即重建（幂等）
SELECT isahl_knowledge.age_rebuild_knowledge_graph();
