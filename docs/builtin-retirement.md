# `builtin` 退役（2026-09-24 落地）

`env-alioth` 过去可以在包内携带一份冻结模型快照（`builtin`，缺省源，零网络）。已退役：
**模型内容一律来自 `ALIOTH_MODEL_SOURCE`，包内只剩"套件"与本插件自己的结构基线。**

## 为什么

包内快照 = 第二份副本 ⇒ 多一次漂移可能。实测的连锁反应：把发行物的 `isahl_meta-registry.sql` 拷进包内后，
它与模型源各持一份；而该文件按约定**不入 Git** ⇒ "干净 clone 缺文件"与"包内必须有文件"互相打架，
先把 vendor 的 sha256 账本（`PROVENANCE.json`）逼成了不可能满足的状态，再让 CI 与本地行为分叉。

## 现在是什么样

| 位置 | 内容 |
|---|---|
| 包内 `packages/alioth/env-alioth/vendor/` | **只留套件与自有资产**：`skill-adapters/`、原型脚本、框架 crates、以及本插件的结构基线 `backend/ddl/002_isahl_meta_schema.sql` |
| 模型内容（物理 DDL、维度种子、注册表行、`latest.json`） | 全部来自 `ALIOTH_MODEL_SOURCE`（`github:owner/repo[@ref]` 或本地路径），缓存于 `<dataRoot>/models` |
| 派生数据（注册表行） | **永不入 Git**（`.gitignore` 按文件名收口）；取不到就忽略（`registrySource: 'missing'`，fk-index 不重生成并如实标注） |

`builtin` 这个源已从代码里删除；`ALIOTH_MODEL_SOURCE` **必填**，缺即启动失败（`requireModelSource()`），
并保留一条响亮的重定向错误：写 `'builtin'` 会被明确告知已退役，而不是被当成路径去解析。

## 部署怎么落地（三处已配置并验证）

`scripts/assemble-model-source.ts`（`mise run alioth:model-source`）把两半合成一个**组装源**：

- **发行物内容**（`latest.json`、`001/002`、维度种子、注册表行）→ 发布根与 `backend/ddl/`；
- **注册表行与结构基线放 `backend/ddl/`**：按文件名即得正确顺序，且旧一代代码（不前置包内基线、按完整路径排序）也吃；
- 组装时**剥掉** `pg_dump` 18 的 psql 专有包封（`\restrict` / `\unrestrict`），旧构建也能直接执行；
- **套件**（`skill-adapters/`，以及 `Pre-Proc/Alioth/_schema/` 若存在）从包内拷入；
- 缺注册表或缺套件 → **非零退出**（不让不完整的源上生产）。

```sh
mise run alioth:model-source -- --source ~/WorkSpace/Alioth --out ~/.dsh-alioth/model-source
# 然后把该目录路径写进部署的 ALIOTH_MODEL_SOURCE：
#   dev  ~/.dsh-alioth.env            m2  launchd 包装脚本        prod  /etc/dsh-alioth/env
```

发布前哨：`~/.dsh-alioth-ops/release.sh` 的 **0d** 相位（`preflight` 自动跑、单相 `sync` 强制）会只读检查目标机
`ALIOTH_MODEL_SOURCE`：缺配置、仍写 `builtin`、或目录不含 `*isahl_meta*.sql` + `skill-adapters/` ⇒ **拒绝发布**。

## 验证（2026-09-24 实测）

| 环境 | 结论 |
|---|---|
| dev | 组装源 + 一次性库：doctor **green**、`model-snapshot 2 isahl_meta DDL, 8 adapters`、注册表 **986 / 31759 / 10.0.34**；`launch` 就绪时表数 0（懒引导）→ 一次碰库后 986 |
| m2 | 同一组装源经 rsync 落地 + env 配置：doctor **green**、`isahl-meta 4 tables`、**986 / 31759**（m2 上仍是旧一代代码 ⇒ 证明布局向后兼容） |
| prod | 组装源投递 + `/etc/dsh-alioth/env` 的 `ALIOTH_MODEL_SOURCE` 已替换（原为 `builtin`）；注册表 sha256 与 dev 一致；服务未重启（`active`） |

测试/自检不依赖网络或真实模型源：`scripts/lib/model-source-fixture.ts` 提供一个夹具组装（包内套件 + 一条微小注册表种子），
`smoke-composition.ts`、`tests/model-surface.spec.ts`、容器 `--check` 都用它。

## 唯一遗留项（模型侧）

发行物仍**不带** `skill-adapters/`（`github:CosmicTools9/Alioth` 的 `main` 与 `v10.0.33` 实测 HTTP 404），
所以组装器要覆盖包内套件。等发行物带上套件（或转为发布资产），组装器可退化为纯拷贝，包内套件随之退役——
那时"单通道"才彻底成立。在那之前：**不要**把模型内容拷回包内（这正是本次退役要消除的做法）。
