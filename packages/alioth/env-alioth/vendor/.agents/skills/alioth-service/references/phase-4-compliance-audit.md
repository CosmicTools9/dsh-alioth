## Phase 4: Service 规约合规审计

> **规则集正本**: `scripts/check/audit-service-spec.ts`（**F1–F7**）。本文件与该脚本逐条一致；两者不一致时以脚本为准。
>
> Phase 4 是交付前最终验证：对目标 Service 单元运行脚本，检查后端分层与架构规约。多数规约在 Phase 1→3 中已自然满足，本阶段捕获遗漏。

### 4-0: 运行审计脚本

```bash
# namespace 内全部 Service 单元
bun scripts/check/audit-service-spec.ts --ns "$NS"

# 单个 Service 单元（--factor = 单元目录 id）
bun scripts/check/audit-service-spec.ts --ns "$NS" --factor "$SERVICE_ID"
```

**服务发现**：`Pre-Proc/{ns}/Sources/Apps/Services` 优先、扁平 `Sources/Services` 回退（`scripts/lib/preproc-layout.mjs`）。**零发现不得空通过**：候选根下存在 `service.json` 却未发现可审计单元（缺 `backend/src`）→ 退出码 1 并列出候选根。

**退出码**：`0` = 全部通过（或该范围内确实无 Service 单元）；`1` = 存在违规或发现失败。

### 4-1: 规则速查（F1–F7，与脚本 `checks` 数组逐条一致）

| 编码 | 规则                                                                                                             | 检测点                                        | 修复位置                |
| ---- | ---------------------------------------------------------------------------------------------------------------- | --------------------------------------------- | ----------------------- |
| F1   | `backend/Cargo.toml` 存在                                                                                        | 路径存在性                                    | Phase 2 2-0 脚手架      |
| F2   | `handlers/mod.rs` 存在，或 `lib.rs` 委托外部 crate（`.configure(` / `::<crate>(cfg` / `::register_service_routes`，聚合服务壳模式） | 文件存在性 / lib.rs 委托判据                  | Phase 2 Handler 层      |
| F3   | `services/` 层无直接 SQL（`sqlx::query\|execute\|fetch\|begin`；依赖注入类型签名合规）                            | grep `services/`                              | Phase 2 Service 层      |
| F4   | `handlers/` 不引用 `models::` 实体（`Request`/`Response`/`Query`/`Record`/`Row`/`Dto`/`Payload`/`Event` 除外）     | grep `handlers/`                              | Phase 2 Handler 层      |
| F5   | 自定义错误类型（`enum *Error` / `struct *Error`）存在时必须用 `thiserror`/`anyhow`；统一 `common::AliothError` 视为通过 | grep `src/`                                   | Phase 2 错误处理        |
| F6   | 业务代码无 `println!`（`src/bin/` 入口输出除外）                                                                  | grep `src/`（排除 `src/bin/`）                | Phase 2 日志（用 `log`）|
| F7   | `handlers/` 无直接 SQL（分层：handlers → services → repositories）                                                | grep `handlers/`                              | Phase 2 Handler 层      |

**不由本脚本覆盖、在 Phase 1/2/3 中承载的设计规则**（勿在 Phase 4 期待脚本报出）：

- DTO 命名（`XxxDetail`/`XxxSummary`，禁 `XxxResponse`）、语义分组 → Phase 2 DTO 设计；
- `qk_*` 标量引用在 DTO 中使用结构化类型（`ScalarPriceValue` 等，非裸 `i64`）→ Phase 2（`DTO_DESIGN_SPEC` §6/§8）；
- 标量解析使用 `resolve_many(&[...])` 批量查询 → Phase 0 批量标量检查 + Phase 2 实现（`DTO_DESIGN_SPEC` §8）；
- `dtoDependencies` 同 namespace 一致性与依赖拓扑无环 → `scripts/check/check-service-dag.ts`（pre-commit 已接入）+ Phase 1-2.5 命名空间校验；
- Block 前端禁 DB 列名 → Phase 3 3-4。

### 4-2: 审计报告

审计输出 Markdown 报告：

```
# {ns}/{factor} 规约审计报告

| 编码 | 结果 | 说明 |
|------|------|------|
| F1 | ✅ | backend/Cargo.toml 存在 |
| F2 | ✅ | handlers/mod.rs 存在 |
| F3 | ✅ | services 层无直接 SQL |
| F4 | ✅ | handlers 未引用 models 实体 |
| F5 | ✅ | 自定义错误使用 thiserror/anyhow |
| F6 | ✅ | 业务代码无 println! |
| F7 | ❌ | `handlers/{entity}.rs:42` 直接 sqlx 查询，改走 services 层 |
| ... | ... | ... |

违规数：1/7（14.3%）
```

### 4-3: 门禁

| 退出码 | 含义                     | 处理                             |
| ------ | ------------------------ | -------------------------------- |
| 0      | 无违规                   | ✅ 通过 — 进入交付               |
| 1      | 存在违规 / 发现失败      | ❌ 阻塞 — 逐条修复后重新审计     |

脚本无 `--skip` 参数；确需临时豁免时按 §Phase 4 报告在 change 内显式记录理由并人工确认（豁免不入脚本）。

---

