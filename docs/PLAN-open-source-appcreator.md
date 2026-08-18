# 行动计划（v2 定稿）：dsh-alioth 成为自足的独立开源 AppCreator 等价物

> **核心事实**（2026-08-18 确认）：
> - AliothStudio 从未开源；`CosmicTools9/AppCreator` 将停止维护
> - dsh-alioth **内置冻结模型工件**——全新机器首次安装零网络
> - 模型演进 = 发布新版插件；不再假设任何未来 GitHub 上游
> - 产物格式与 AliothStudio/Gateway 兼容（纯消费者，无上游写）

## vendor 集（定稿，~10.7MB，全量保留）

| 工件 | 体积 | 判定 |
|---|---|---|
| `backend/ddl/*isahl_meta*.sql` | 6.3MB | vendor（注册表基线） |
| `skill-adapters/*.yaml` | 40K | vendor（9 轨道定义） |
| `scripts/prototype-tool.js` + `check/audit-css-framework.mjs` + `eval/evaluate-prototype-reference.ts` | ~150K | vendor（构建链三件） |
| `.agents/skills/alioth-design/references/**`（含 vendor/ UMD 资产） | ~4.2MB | vendor（shell 模板运行时依赖） |
| Rust crates / worker / 生成器 | — | **不 vendor**（TS 等价已闭环，无运行时角色；版本锚点固化常量 `"10.0.0"`） |

## 合规（定稿）

- 分发树工件 Apache-2.0 再分发 + The Alioth Authors 署名（NOTICE）
- **第三方声明自备**：references/vendor 的 React/react-dom/babel 为 MIT（NOTICE 条目）；alioth-components 为 Alioth 自产（Apache-2.0 主声明）
- 语义模型（bge ~90MB）：不 vendor；release asset 可选下载 + 无模型时 semantic_search 降级字面查询（schema_info 常可用）

## 执行阶段

1. **vendor 化**：工件复制 → `env-alioth/vendor/`；`modelSource` 默认 `builtin`；版本锚点固化
2. **gate 链接线**：workflow program runner cwd → vendor 根
3. **零网络验收**：全新 dataRoot + 断网模拟 → doctor 全绿（内置 PG + 冻结模型，无网络请求）
4. **文档/合规**：AGENTS 冻结定位 + NOTICE 第三方 + README + 排查无上游假设
5. **端到端**：真实对话全链路（含原型 gate）+ 导入手动验收
6. **发布**：v0.1.0 + 发布物清单
