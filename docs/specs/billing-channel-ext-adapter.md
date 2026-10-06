# 平台商务 ↔ NS:Cosmic-Tools 的对接形状（billing 通道 / L-EXT 镜像规约）

> 2026-10-07 · 架构评审「C4」的落地契约。上游正本：AliothMeta `docs/specs/EXTERNAL_INTEGRATION_SPEC.md`（L-EXT 外部镜像 schema、L-ADP 适配器、L-ISahl 冻结、L-MAP 双向转换）。

## 两个「订单」不得混淆

| 主体 | 数据落点 | 注册方式 |
|---|---|---|
| **平台自己的商务**（L1 订阅、L2 源码授权、账单、发票、用量） | dsh-alioth 自有 schema `dsh_alioth_billing` | 永不进入 isahl / isahl_meta（铁则「任何情况下写 schema `isahl`」+ 2026-09-24 注册表出库裁决） |
| **用户生成的应用**的业务实体（订单/库存/财务……） | 各 namespace 工作区 + `dsh_alioth` 注册表 | `alioth_entity_write` → `isahl_meta.meta_collections/meta_fields`，实体挂模型继承族（订单挂 `zc_id_stat-trade_order` 家族等） |

模型侧**现成可复用**（v10.0.38 实测）：订单/财务表族（`zc_id_stat-trade_order`、`zc_id_bill`、`zc_id_invoice`、`zc_id_stat-bok-voucher`、`zc_id_appr-payment` 等）+ 订单/财务/支付坐标码（scene `KB 在线支付`、factor `FJA 交易订单`/`FMC 付款计划` 等）。模型**没有**支付网关/流水表——外部支付按上游规约走 ext 镜像。

## 支付通道：本仓的缝 + NS 侧的适配器

dsh-alioth 定义**结算缝**，不做 PSP 协议：

1. `aliothBilling.applyChannelPayment(channel, billReference, amountCents, note)` —— 渠道把已结算的账单引用提交进来；金额必须与应收一致（不符 = 409，是**对账事件**不是支付）；幂等（重复回调返回同一 `paid` 结果）。
2. HTTP 边缘 = billing-web-alioth 的 `POST /api/billing/channel/<channel>/callback`：
   - 头 `x-alioth-signature: t=<unix>,v1=<hex>`，`v1 = HMAC-SHA256(ALIOTH_BILLING_CHANNEL_SECRET, "<t>.<rawBody>")`，时间戳窗口 ±5 分钟；
   - 体：`{"billId": "…", "amountCents": 139900, "note": "wechat-pay:trade_no=…"}`；
   - `ALIOTH_BILLING_CHANNEL_SECRET` 未配置 ⇒ 端点整体 503（fail-closed）。
3. **适配器（wechat-pay / alipay 等）住在 NS:Cosmic-Tools 的 `Pre-Proc/Cosmic-Tools/Ext-adapter/` 面**（上游 L-EXT/L-ADP；NS 仓已留 `wechat-pay` 槽位）：它持有 PSP 密钥、处理回调验签与对账，结算完成后按上述契约回调本服务。数据镜像按上游命名 `<ns小写>_<ext>` / 通用 `ext_wechat_pay`，经 L-MAP 双向转换，不进 isahl。

## 身份对接（C3，同一次评审的姊妹决策）

dsh-alioth 的身份缝提供 `authMode: 'oidc'` 适配器：本平台作为 **OIDC RP** 对接 NS:Cosmic-Tools 的 embedded SSO（`Deploy/sso-mode.conf: Cosmic-Tools embedded`，SSO 自建 OIDC Provider + SCIM + MFA/重置/三方登录）。边界：`isahl_auth` 永远归 SSO 运行时写，本平台只经 OIDC/经 SSO API 消费——与「不直写 `isahl`」同款的文字边界，同样铁律化。

## 运营速查

| 操作 | 命令 / 入口 |
|---|---|
| 开通/续期 L2 | `pnpm run source:grant --user <名> --until YYYY-MM-DD` 或 admin 页 |
| 管理员授予/撤销 | `pnpm run admin:grant --user <名> [--revoke]`（`--list` 清点） |
| 对账 | admin 页「对账」面板（`reconcile`：卡单 / 已付无单 / 单据缺失） |
| 计费事件通知 | env `ALIOTH_NOTIFY_WEBHOOK`（fire-and-forget webhook） |
| 计量预算 | env `ALIOTH_LLM_MONTHLY_COST_CENTS_L0 / _L1`（未设 = 只计量不判上限） |
| BYOK | env `ALIOTH_BYOK_SECRET`（未设 = 用户自存 key 功能关闭） |
