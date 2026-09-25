-- 038_age_ngac_projection_rebuild_only.sql
-- NGAC AGE 图投影：写路径去 cypher（rebuild-only 模式）
--
-- 背景（2026-09-19/20 实测，PG 18.6 + age 1.8.0 源码构建）：
-- 037 安装的五个行级触发器在写路径内调用 ag_catalog.cypher()，可致后端进程
-- **段错误**（PG 日志 `client backend (PID …) was terminated by signal 11`）
-- → `terminating any other active server processes`（全簇 reinit，并行会话连接
-- 被掐断）。崩时语句已取证：`seed-cosmic-app-visibility.sql` 的
-- `INSERT INTO isahl_auth.ngac_association` → 触发器 ngac_age_sync_assoc
-- → age_sync_association()。PG 日志累计 45 次（2026-09-19 19:xx 9 次 / 23:xx 36 次）。
-- 崩溃发生在 C 层，PL/pgSQL 的 EXCEPTION 捕获不到 ⇒ 037 设计的
-- 「同步失败不阻断主写（fail-open）」语义在该宿主组合下失效（进程级崩溃).
--
-- 处置：NGAC 写路径 MUST NOT 触发 AGE cypher。图退化为 rebuild-only：
--   * 权威源 = `ngac_*` 关系表（不变，NGAC_SPEC §11）；
--   * 对账 = isahl_auth.age_ngac_projection_diff()（纯 SQL，不变）；
--   * 重建 = isahl_auth.age_rebuild_ngac_graph()（O(n) SQL 直插，不变）；
--   * 触发 = SSO/Gateway 启动钩子（`age_projection.rs`：needs_heal 对账漂移非零即重建）。
--
-- 顺序契约：本迁移幂等，且 **MUST 在 037 之后应用**——037 会重建触发器，故
-- 「037 → 038」是唯一合法顺序；运行时自愈链（age_projection.rs::heal）按此序应用
-- （两文件均为 `ngac/sql/` 内嵌资产，随 crate 编译）。
--
-- 影响面：投影由「同事务强一致」改为「对账驱动的最终一致」。决策热路径不读图
-- （NGAC_SPEC §11.2 红线）、graph_snapshot 走关系单源 ⇒ 无消费方受影响；
-- 图的新鲜度语义见 NGAC_SPEC §11.1。

-- ═══ 1) 停用全部 AGE 同步触发器（按函数名模式匹配，抵御触发器名漂移）═══
DO $fix$
DECLARE
    r record;
    n integer := 0;
BEGIN
    -- 不按 AGE 扩展存在性早退：DROP TRIGGER 不依赖 AGE；无 AGE 环境下 037 仍已
    -- 建出触发器（函数体懒规划不校验 ag_catalog），跳过此处会让 §2 退役函数撞
    -- 「cannot drop function … other objects depend on it」。
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'age') THEN
        RAISE NOTICE 'age 扩展未安装，仍按模式停用同步触发器（DROP TRIGGER 无需 AGE）';
    END IF;
    FOR r IN
        SELECT t.tgname, c.oid::regclass AS tbl
          FROM pg_catalog.pg_trigger t
          JOIN pg_catalog.pg_class c ON c.oid = t.tgrelid
          JOIN pg_catalog.pg_proc p ON p.oid = t.tgfoid
         WHERE NOT t.tgisinternal
           AND c.relnamespace::regnamespace::text IN ('isahl_auth', 'isahl_knowledge')
           AND p.proname LIKE 'age\_sync\_%'
    LOOP
        EXECUTE format('DROP TRIGGER IF EXISTS %I ON %s', r.tgname, r.tbl);
        n := n + 1;
    END LOOP;
    RAISE NOTICE '已停用 % 个 AGE cypher 同步触发器（rebuild-only 模式）', n;
END;
$fix$;

-- ═══ 2) 退役同步函数本体（零调用方；保留则可能被误重建触发器重新武装）═══
DO $fix$
DECLARE
    r record;
    n integer := 0;
BEGIN
    FOR r IN
        SELECT p.oid::regprocedure::text AS sig
          FROM pg_catalog.pg_proc p
          JOIN pg_catalog.pg_namespace ns ON ns.oid = p.pronamespace
         WHERE ns.nspname IN ('isahl_auth', 'isahl_knowledge')
           AND p.proname LIKE 'age\_sync\_%'
    LOOP
        EXECUTE format('DROP FUNCTION IF EXISTS %s', r.sig);
        n := n + 1;
    END LOOP;
    RAISE NOTICE '已退役 % 个 AGE cypher 同步函数', n;
END;
$fix$;

-- ═══ 3) 投影一致性基线：重建一次并记录漂移（纯 SQL，无 cypher）═══
DO $fix$
DECLARE
    v_drift bigint := -1;
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_extension WHERE extname = 'age') THEN
        RETURN;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM ag_catalog.ag_graph WHERE name = 'isahl_auth') THEN
        RETURN;
    END IF;
    PERFORM isahl_auth.age_rebuild_ngac_graph();
    SELECT count(*) INTO v_drift FROM isahl_auth.age_ngac_projection_diff() d WHERE d.drift <> 0;
    RAISE NOTICE 'AGE 投影重建完成（rebuild-only）：对账漂移行 = %', v_drift;
END;
$fix$;
