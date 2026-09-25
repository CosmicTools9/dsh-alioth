# 把 dsh-alioth 的注册表搬出模型样例库（2026-09-24）

## 为什么

2026-09-15 的部署约定让 dsh-alioth 与模型样例共用 `alioth` 库（同机同 PG、同库不同 schema）。
那是错的：`isahl_meta` 这个命名空间**模型自己也拥有**，把本插件的注册表建在里面，等于让
baseline 的 `CREATE`/`DROP … RESTRICT` 修复、`resetRegistry()` 的 `DROP SCHEMA … CASCADE`
和本插件的表，都落在模型的对象旁边（实测：dev 的 `alioth` 库里出现过本插件建的
`meta_collections`/`meta_fields`，而该库 `isahl_meta` 中另有模型侧的孤立对象）。

现在代码 fail-closed 拦死：`env-alioth` 的 `bootstrapDatabase()` 每次调用先查 catalog，
目标库 `isahl` schema 有普通表即判为「模型库」并拒绝引导（`assertNotModelDatabase`）。
**症状**：带 guard 的代码 + 未搬库的 DSN ⇒ `/api/auth/login` 等一切库面调用都回
`env-alioth: database "alioth" holds the Alioth model (schema \`isahl\`, N table(s)) — refusing to bootstrap…`，
登录页会把这行原样显示（2026-09-24 dev 现场）。

**正确拓扑**：dsh-alioth 的注册表在它自己的库 `dsh_alioth`（同名于它的四个 schema）；
`alioth` 库归模型/NS:Alioth（`isahl` 是其业务表空间，本插件只读，连它都只是历史习惯）。

本文的 SQL/命令**由你执行**（仓库规约：agent 不代执行 DB 域批量操作）。代号：
`$MODEL` = 模型样例库（如 `alioth`），`$REG` = 新的注册表库（`dsh_alioth`）。

验收基线（本文命令已在 dev 的**一次性库**上跑通，见文末「实测证据」）：
`$REG` 首次 boot 从模型快照重建注册表 = **986 collections / 31759 fields**（模型 v10.0.34），doctor `status green`。
（2026-09-24 19:0x 起包内冻结副本已换成发行物 sidecar；此前实测为 905 / 27727 @ v10.0.0——见文末「实测证据」。）

## 0. 先看清单（只读，两个库都跑）

```bash
# 本插件在模型库里的对象（自有 schema + isahl_meta 里的 baseline 残留）
psql "$MODEL" -X -c "select n.nspname, c.relkind, c.relname, pg_get_userbyid(c.relowner) owner from pg_class c join pg_namespace n on n.oid=c.relnamespace where n.nspname like 'dsh\_alioth%' or n.nspname='isahl_meta' order by 1,2,3"
psql "$MODEL" -X -c "select p.oid::regprocedure fn from pg_proc p join pg_namespace n on n.oid=p.pronamespace where n.nspname='isahl_meta'"
psql "$MODEL" -X -c "select t.typname from pg_type t join pg_namespace n on n.oid=t.typnamespace where n.nspname='isahl_meta'"

# 模型库里的注册表**有没有非 builtin 的内容**（有 = 必须搬，见第 2 步）
psql "$MODEL" -X -Atc "select count(*) from isahl_meta.meta_collections"
psql "$MODEL" -X -Atc "select md5(string_agg(table_name, ',' order by table_name)) from isahl_meta.meta_collections"
```

- `meta_collections` **不存在**（表被外部清掉过，只剩 `devv_*` 视图 / 3 个枚举 / `gf_*` 函数）⇒ 没有内容要搬，
  新库自己 bootstrap 即可。2026-09-24 实测 dev 即此形态。
- 计数 **905** 且 md5 = `4846ba0e7678d26bf3d2a1553a6d5ba2` ⇒ 就是 builtin 种子，**同样没有内容要搬**。
  2026-09-24 实测 m2 的 `alioth` 即此形态。
- 计数/指纹不等 ⇒ 库里有本地注册的自定义实体（`alioth_entity_write` 写进去的），必须按第 2 步搬走，
  否则新库 bootstrap 出来的注册表会丢掉它们。
- 清单里出现任何你没预期的对象 → 先停下人工判断，别照抄下面的 DROP。

## 1. 建专用库并授权

```bash
psql postgres -X -c 'CREATE DATABASE dsh_alioth OWNER alioth'   # role 按你的部署改；owner 即后续 DSN 的 role
# 仅当 owner ≠ DSN role 时才需要：
psql dsh_alioth -X -c 'GRANT CREATE, CONNECT ON DATABASE dsh_alioth TO alioth'
```

owner 隐式持有 `CREATE`/`CONNECT`，所以「用同一 role 建库」时第二条不用跑。
（本仓的 dev/m2/prod 三处 DSN role 都是 `alioth`：`alioth` 是普通 role，**自己建不了库**，
`CREATE DATABASE` 要用超级用户跑。）

## 2. 搬走本插件自己的 state

**账号与授权必须搬**（`dsh_alioth_auth` 是用户账号，`dsh_alioth_billing` 是已付费的 L2 授权）：

```bash
pg_dump "$MODEL" --no-owner --no-privileges \
  -n dsh_alioth_auth -n dsh_alioth_billing \
  | psql "$REG" -v ON_ERROR_STOP=1
```

**注册表本体**：第 0 步指纹 = 发行物 builtin 时**跳过**（新库首次 `ready()` 自己跑 baseline + 种子，
产出同样的 986 / 31759）；只有存在自定义实体时才连 `-n isahl_meta` 一起 dump，
并且要在**新库第一次 `ready()` 之前**恢复完——`meta_collections` 已在 ⇒ bootstrap 走 **adopt**（`created=false`），
晚一步就会先被 baseline 建出来、恢复撞上「已存在」。

**不要搬 `-n dsh_alioth`**：那是 boot 出处的印章（`model_state`），不是数据。新库首次 boot 自己写；
搬过去只会让 doctor 拿旧印章比对、报一次无意义的 drift。

```bash
psql "$REG" -X -Atc "select count(*) from dsh_alioth_auth.users"   # 应与 $MODEL 侧一致
```

## 3. 从模型库摘除本插件的对象（单事务，全 RESTRICT）

只摘**自有 schema**；`isahl_meta` 里的对象**不要动**：

```sql
BEGIN;
DROP SCHEMA IF EXISTS dsh_alioth_auth CASCADE;
DROP SCHEMA IF EXISTS dsh_alioth_billing CASCADE;
DROP SCHEMA IF EXISTS dsh_alioth CASCADE;
COMMIT;
```

（确认已搬到 `$REG` 之后再删。`CASCADE` 只作用于这三个 schema 内部。）

**为什么不动 `isahl_meta`**：那个 schema 及其 `devv_inherits_*` 视图 / `gf_*` 函数 / 3 个枚举，
`002_isahl_meta_schema.sql`（baseline）会建，但**模型侧同样依赖它们**——模型发行物的
`002_isahl_tables.sql` 自己就引用 `isahl_meta.meta_collections`、`isahl_meta.devv_inherits_view`、
`isahl_meta.gf_query_inherits`。在模型库里删掉它们等于削 NS:Alioth 的查询面，而且对本插件**毫无收益**：
guard 只看 `isahl`（业务表空间）在不在，残留的 `isahl_meta` 不会被当成「我们还在用模型库」。
遗留的孤立枚举（如 `isahl_meta.zc_id_unit_formatter_enum`）同理，本仓代码里根本没有它。

真要清理时先取证依赖：

```bash
psql "$MODEL" -X -c "select distinct dn.nspname||'.'||dc.relname dependent from pg_depend d join pg_class c on c.oid=d.refobjid join pg_namespace cn on cn.oid=c.relnamespace join pg_class dc on dc.oid=d.objid join pg_namespace dn on dn.oid=dc.relnamespace where cn.nspname='isahl_meta' and dn.nspname<>'isahl_meta'"
```

2026-09-24 实测 dev 的 `alioth` 返回 0 行（该样例的 `isahl` 里一张视图都没有），m2/prod 未测——所以这条仍是「有依赖就别删」的判断题，不是必做步骤。

## 4. 改 DSN 并重启

```bash
# dev：~/.dsh-alioth.env（0600，由 ~/.zshenv 引入）
ALIOTH_DATABASE_URL=postgres://alioth@127.0.0.1:5432/dsh_alioth
# m2：~/bin/dsh-alioth-web.sh（launchd 包装脚本）里的同一个变量
# prod：/etc/dsh-alioth/env（systemd EnvironmentFile，0600 root）
# 容器：不用改——入口脚本会自建 dsh_alioth，旧卷把 alioth 就地 ALTER DATABASE … RENAME
```

**源也必须给**（2026-09-24 起 `builtin` 退役）：每台机
`mise run alioth:model-source -- --source <模型发行物目录> --out ~/.dsh-alioth/model-source`，并把 `ALIOTH_MODEL_SOURCE` 指向它（
与 DSN 同一个 env 文件：dev/m2 的 `~/.dsh-alioth.env`、prod 的 `/etc/dsh-alioth/env`）。侧车行不在 GIT 里，
发行物内容（含 `isahl_meta-registry.sql`）要随发布 out of band 带到目标机；否则 boot 报 `ALIOTH_MODEL_SOURCE is required`。

**顺序要紧**：带 guard 的代码**先上线后搬库 = 全站库面不可用**（登录页直接显示 refusal）。
prod/m2 上线新代码前必须先跑完本文第 1–4 步。

## 5. 验收

```bash
psql "$REG"   -X -Atc "select 'collections='||(select count(*) from isahl_meta.meta_collections)||' fields='||(select count(*) from isahl_meta.meta_fields)||' users='||(select count(*) from dsh_alioth_auth.users)"
psql "$MODEL" -X -c "select nspname, count(*) from pg_class c join pg_namespace n on n.oid=c.relnamespace where n.nspname in ('dsh_alioth','dsh_alioth_auth','dsh_alioth_billing') group by 1"
mise run alioth:doctor            # exit 0 = 绿
```

期望：`986 / 31759` + 原有账号数；模型库里三个自有 schema 已消失（`isahl_meta` 保留）；
doctor `boot created=true stamped=true`、`✓ isahl-meta  4 tables incl. meta_collections, meta_fields`、
`✓ semantic-index  … entries`、`status green`。然后**走真实面**验一次：`/login` 用原账号登录
（旧 DSN 下同样的请求会回 refusal 横幅，这就是判据）。

反向验证（应当被拒）：把 DSN 临时指回 `$MODEL` 再 `mise run alioth:doctor` →
应报 `holds the Alioth model (schema "isahl", N table(s)) — refusing to bootstrap…`。

## 实测证据（2026-09-24，dev）

在一次性库 `dsh_alioth_migration_dryrun` 上跑完整套（不碰 `alioth`，跑完 DROP）：

1. `pg_dump alioth -n dsh_alioth_auth -n dsh_alioth_billing | psql <dry>` → 6 users 恢复。
2. `alioth-doctor` → `boot created=true stamped=true`、`isahl-meta 4 tables`、`semantic-index 5826 entries`、`status green`；
   注册表容器 = **905 collections / 27727 fields**（当时的 builtin 冻结种子，指纹与 m2 的 `alioth` 相同 `4846ba0e…`；
   包内副本换成发行物 sidecar 后，同样的流程得到 986 / 31759）。
3. 挂真插件（`env-alioth` + `auth-alioth`，`mode: enforce`，preProc/deploy root 指 `/tmp`）跑 6 个搬迁账号：
   `login('isahl', 错的密码)` → `aliothAuth.login: invalid credentials`（证明行读到了、scrypt 比对真的跑了，
   而不是「表不存在」那种错）；`register → login → userForToken → workspaces` 全通。

## 回滚

搬回只需反向 dump/restore（`$REG` → `$MODEL`）并改回 DSN；但 guard 会拒绝在模型库里引导，
所以「回滚」只在旧版本代码（无 guard）下才成立。建议保留 `$REG` 的 dump 作为回滚点。
