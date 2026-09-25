# isahl_meta 注册表：从哪来、什么时候自愈、什么时候必须显式重建

面向**操作者与维护者**。文中每个断言都能用给出的命令复现；数字取自 2026-09-24 的实测。
模型库↔注册表的耦合事实与逐库处置见 `~/.dsh-alioth-ops/db-topology-notes.md`（机器本地）；
把注册表搬进自有库的步骤见 [`migrations/2026-09-24-registry-out-of-model-database.md`](migrations/2026-09-24-registry-out-of-model-database.md)。

## 1. 三条结论

1. **引导只来自模型快照**：`env-alioth` 解析快照 → 结构基线（包内冻结的 `002_isahl_meta_schema.sql`）
   ＋ 注册表行 → 在**一个往返事务**里执行。不探测、不借用别的库。
2. **缺表会自动自愈**（`launch` / `alioth:doctor` / 任何首次碰库都是同一个函数）；**模型演进导致的陈旧不会自动重跑**
   （基线是 load-once 契约）→ 需要显式 `--reset`。
3. **模型库一律拒绝**：`schema isahl` 里有普通表的库 = 模型库（`assertNotModelDatabase`，每次引导都先查）。
   `launch` 与 `alioth:doctor` 走**同一个**函数，不存在"哪个能绕过"。

## 2. 三种状态 × 两条路径

| 库的状态 | `mise run launch` | `mise run alioth:doctor` |
|---|---|---|
| 空库／注册表表被外部删掉 | **自愈**（懒触发，见 §3） | 同样自愈，并打印 provenance / drift |
| 注册表在，但模型演进了（枚举/集合变了） | 不重跑，只报告 drift | 同样不重跑；要真重建 → `--reset` |
| 目标是**模型库** | 服务能起来；**真正碰库的那个请求**失败 | 失败更早也更清楚：错误文本 + 非零退出码 |

## 3. 自愈是懒的：服务就绪 ≠ 已引导

实测（全新空库 + 真实控制台，`:3160`）：

```
server serving        — 此时 isahl_meta 表数: 0        # ready() 尚未被调用
POST /api/auth/login → 401                            # 一次“碰库”
isahl_meta 表: meta_collections, meta_fields
collections=986 fields=31759 model_version=10.0.34 auth_schema_tables=2
```

（同一实测在 2026-09-24 18:53 发行物注册表被 vendored 进包内之前跑出的是 `905 / 27727 / 10.0.0`——
即那时的缺省 `builtin` 还在用旧的冻结种子。**注册表规模会随模型源变化**，见 §4。）

`ready()` 是记忆化的，**首次 `sql()` 才触发**引导（`src/index.ts`）；因此：

- 只想"它自己修好" → `mise run launch` 就够；
- 想"现在就看到结果/看它是否绿" → `mise run alioth:doctor`；
- 只想看界面是否活着 → 不算数：`GET /` 与登录一个不存在的用户都**不碰库**（实测在模型库 DSN 下也照常返回 404/401）。

## 4. 注册表内容取决于模型源

三种源的实际内容（2026-09-24 实测；`builtin` 已退役，源**必需**）：

| 源 | 注册表行 | 套件（`skill-adapters/`） | 结论 |
|---|---|---|---|
| **组装源**（部署用；`mise run alioth:model-source`） | 发行物那份（986 / 31759，`v10.0.34`） | **8 个** | ✅ 唯一完整的形态：内容 + 套件同处一目录 |
| 原始模型树（如 `~/WorkSpace/Alioth`） | 986 / 31759 ✓ | **0** ✗ | 缺套件 ⇒ 工作流/编排缺面；只适合做字典/内容来源 |
| GitHub 克隆（`main` / `v10.0.33`） | 无（HTTP 404，实测） | 无（HTTP 404，实测） | 两者都缺 ⇒ 只适合字典新鲜度校验 |

组装器（`scripts/assemble-model-source.ts`）把两半合成到 `<out>`：发行物内容（`latest.json` + 注册表行 + 模型 DDL/种子）
放发布根，**注册表行与结构基线放 `backend/ddl/`**（两种代码世代都读、按文件名即得正确顺序），再从包内拷入套件。
缺注册表或缺套件时它**非零退出**——不会让一个不完整的源悄悄上生产。

`registrySource`（`snapshot` / `missing`）与 `modelVersion` / `sourceRef` 都会出现在 `ctx.aliothEnv.ready()`
的返回值与 doctor 报告里——排查"行数不对"时先看这三个字段。

`registrySource`（`snapshot` / `missing`）与 `modelVersion` / `sourceRef` 都会出现在 `ctx.aliothEnv.ready()`
的返回值与 doctor 报告里——排查"行数不对"时先看这三个字段。

## 5. 发行物契约（消费端要能吃得下的形态）

- **只带行，不带结构**：结构恒用包内冻结基线（`env-alioth/vendor/backend/ddl/002_isahl_meta_schema.sql`）。
  快照内（发布根或 `backend/ddl/`）**文件名含 `isahl_meta`** 的 `.sql` 会被取用，按文件名排序执行
  ⇒ `002_isahl_meta_schema.sql` 一定排在 `isahl_meta-registry.sql` 之前。
  组装源**两者都带**（基线随源走）：旧一代代码不前置包内基线、且按完整路径排序，源自带基线才两种世代都对。
- 必须是 **INSERT** 形态（`pg_dump --inserts`）。`COPY … FROM stdin` 会被**响亮拒绝**——
  否则"加载 0 行"会看起来像一次成功引导。
- PostgreSQL 18 的 `pg_dump` 会在首尾加 psql 专有元命令 `\restrict <token>` / `\unrestrict <token>`；
  消费端会剥掉（`src/registry-ddl.ts`），且只在**字符串字面量之外**剥——`--inserts` 的值可能跨行并以 `\` 开头，那是数据。
- **结构面（三个枚举）目前只活在包内基线里**：模型每扩枚举，基线必须同步。2026-09-24 已补
  `field_data_type` 的 `coordinate, path, area, circle, polygon`（模型侧新增的地理类型），并同步 vendor 基线（含 `field_data_type` 枚举扩展）。
- **注册表行是派生数据，不入 Git**：既不提交，也不作为包内副本长期存放——多一份拷贝就多一次漂移可能。
  `.gitignore` 按文件名收口（`isahl_meta-registry.sql`），所以它对任何落点都生效。
- **探测即忽略**：消费端只从**模型源**取它（`<模型源>/isahl_meta-registry.sql`，扁平/版本目录两态都认），
  取不到就**忽略**——运行时退化成 `registrySource: 'missing'`（告警照常启动，注册表类工具各自报错），
  字典链的 `fk-index` 则不重生成、并在门禁输出里如实标注"no registry sidecar, anchored bytes only"
  （已锚定的字节仍然受校验，绝不静默当作通过）。
- `check:vendor` 的 **sha256 清单（`PROVENANCE.json`）已于 2026-09-24 退役**：它把"本就不提交的文件"记进账本，
  干净克隆上必报 `manifest entry without file`；vendor 变更还得手工改一个派生文件。现在它只校验
  LICENSE / NOTICE 合规；新鲜度交给 `sync:framework --check`（对 AliothStudio 源码）与 `check:dicts`（对模型源）。

## 6. 操作手册

```sh
mise run launch                    # 起控制台（GUI）；缺表会在首个碰库请求里自愈
mise run alioth:doctor             # 显式自检：快照 → 连接 → 引导 → 健康报告（退出码 0 = 绿）
mise run alioth:doctor --reset     # 重引导：清 isahl_meta + dsh_alioth 两处，再按当前模型重建
                                   #   不动 dsh_alioth_auth（账号）与 dsh_alioth_billing（已售授权）
mise run alioth:rebuild-semantic   # 强制重建语义索引（entriesHash 变化本来就会自动重建）
mise run alioth:model-source -- --source ~/WorkSpace/Alioth --out ~/.dsh-alioth/model-source
                                   # 组装模型源（发行物内容 + 包内套件）；`--fixture-seed` 可造自检用夹具
```

环境覆盖：`ALIOTH_DATABASE_URL`（必需）与 `ALIOTH_MODEL_SOURCE`（**必需**——`builtin` 已退役，缺即启动失败）、
`ALIOTH_DATA_ROOT`、`ALIOTH_PRE_PROC_ROOT`。模型源应指向**组装源**（含 `*isahl_meta*.sql` 与 `skill-adapters/`）；
注册表行由模型发布管线生成、不入 Git，取不到就退化（见 §5），不阻塞启动但注册表类工具不可用。
`--reset` 会清掉本地注册的自定义实体，执行前请确认。

```sh
ALIOTH_REPO=~/WorkSpace/Alioth pnpm run check:dicts   # 字典新鲜度（无 sidecar 时 fk 部分如实跳过）
pnpm run check:vendor                                 # vendor 合规（LICENSE/NOTICE）
```

## 7. 故障对照

| 现象 | 含义 | 处置 |
|---|---|---|
| `refusing to bootstrap this plugin's registry inside it` / `holds the Alioth model (schema isahl, N table(s))` | DSN 指向模型库 | 迁到自有库并改 DSN（见 migrations 文档） |
| `already has an isahl_meta schema holding N table(s) but no meta_collections` | 那是**别人**的注册表，本插件拒绝猜 | 指到空库/自有库，或先清掉外来 schema |
| `关系 "isahl_meta.meta_collections" 不存在`，报错来自模型侧函数（如 `isahl.gf_seed_tables()`） | **模型库**缺它自己的注册表（模型的函数在读它） | 模型侧补注册表；与 dsh-alioth 的注册表无关，别用本插件去补 |
| 控制台能开、某个操作 500 | 懒引导在那个请求里才失败 | 看日志里的 `env-alioth:` 行；先跑一次 `alioth:doctor` |
| `ALIOTH_MODEL_SOURCE is required` | `builtin` 已退役、且未配置模型源 | `mise run alioth:model-source -- --source <模型发行物> --out <dir>`，把 `<dir>` 写进部署 env |
| `model source 'builtin' was retired` | 配置里仍写着退役的源 | 同上；该错误是刻意响亮（否则会被当成路径去解析） |
| 行数从 905 变/不变 | 看 `registrySource` 与 `modelVersion`（§4） | 发行物带 `isahl_meta-registry.sql` ⇒ 986 |
| `registrySource: 'missing'`、注册表类工具报错 | 模型源没带 `isahl_meta-registry.sql` | 给模型源带上该 sidecar（带外投递；不要拷进本包，避免第二份副本漂移） |
