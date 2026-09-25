# 库存服务实现契约（Inventory Service Contracts）

> **状态**：2026-08-12 沉淀（WZ 运输链实施验证）。配套模型契约：`alioth-ontology/references/inventory-model-design.md`（凭证族/储元族/产品族/状态族/交易流）。
> 本文件为 **Service 实现层**契约：凭证写路径、读侧校验口径、状态机、fk_index 注册、测试模式与陷阱清单。
> 参考实现：`Pre-Proc/WZ/Sources/Apps/Services/{transport-dispatch,transport-operations}`（commit 57f09a821 / c2b6f45c7 / 17b7bc3ba）。

## 1. 凭证写路径（方向性单行）

每个库存动作写**方向性单行**凭证：源池行 `qk_outgo` + `ck_sto-title`=源池类目；目标池行 `qk_income` + `ck_sto-title`=目标池类目。禁止单行同时承载双向（`ck_sto-title` 单值列无法表达池对）。

| 动作 | 凭证 | 写点 | 行 | 池对（源→目标） |
|---|---|---|---|---|
| 商品/服务上架 | `zc_id_stat-slf-voucher` 实例 | 容量配置端点 | 1（qk_income） | 货架 |
| 物流委托（交易发生） | `zc_id_stat-com-voucher` 实例 | 委托创建事务 | 2（OUT/IN） | 货架→履约 |
| 订单受理 | `zc_id_stat-tsp-voucher` 实例 | **委托级**受理动作 | 2 | 履约→在途 |
| 派车 | `zc_id_stat-tsp-voucher` 实例 | **派车事务**（每车） | 2 | 在途→在途 |
| 签收/完结 | `zc_id_stat-tsp-voucher` 实例 | 签收事件 | 2 | 在途→空闲 |

**标准列**：`fk_production`（容量池产品）、`fk_subj-storage`/`fk_obj-storage`（线路储元，运输域空间载体）、`qk_income`/`qk_outgo`/`qk_total`（标量引用）、`ck_sto-title`（池类目，4 池种子 STO-SHELF/FULFILL/TRANSIT/IDLE，叶表 42）、`dk_scene/dk_factor/dk_function`（坐标）、`_t_`='实例'。量值一律存标量引用 ID，**禁止对 qk_* ID 做算术**（读取必须 JOIN `zc_id_scale`/`zc_id_scal-*` 取 mark）。

**叶表铁律**：INSERT 只落叶表（com/slf/tsp/smt-bank|cash|channel）；`sto-voucher`/`smt-voucher` 为中间层。凭证 `comments` 仅承载扩展语义（type/direction/family/tier），禁承载量值。

## 2. 运力池语义与读侧口径（2026-08-30 更新）

**运力池 = 当前可调度运力**：池 = (容量池产品 × 线路储元) 库存行，`qk_qty`（mv_inventory.qty）= 当前可调度余量，`qk_p_capacity`（mv_inventory.capacity）= 总容量上限；`used = capacity − qty`。

**消耗-恢复周期匹配（而非直接对齐）**：池的消耗（下单整单扣可售）与恢复（签收/取消回补）构成周转周期，与委托要求周期匹配即可；**不要求池余量逐委托/逐时点直接对齐**（无在途总量镜像校验、无按周期锁定）。

- **可售扣减 = 下单时点一次整单扣减**（`COM-{code}-OUT` 凭证物化，守卫 `__min=0` + `__max=容量上限` 防超卖）。
- **派车 = 形态迁移**（tsp 凭证在途→在途），**不再做池总量/余额校验**——原「派车可用量 = 在途范例余额」读径（transit_net）已退役（受理预留双计/字典缺失静默失效/负余额虚增三问题）。
- **恢复 = 回补可售**：签收 `TSP-SGN-{wb}` / 取消运单 `TSP-RLS-WB-{wb}` / 取消委托 `TSP-RLS-C-{id}` 三路径；净额核算幂等（`reservation − Σ已回补`，≤0 不写）；**回补 MUST NOT 传 `__max`**（修正语义——被上界拒绝会阻断签收/取消；漂移由净额核算与对账暴露）。
- **读径统一**：列表/详情/线路聚合均按 mv_inventory 单一口径实算，禁车辆载重 SUM 回退与 tsp/legacy 聚合。
- **legacy**：存量 `dispatch_deduction` comments-JSON 凭证随 transit_net 退役，不再参与读径。
- 超卖阈值：下单守卫 `__min`/`__max` 为硬阈值；受理前置派车为既定流程（前端按钮状态门禁约束）。

## 3. 状态机契约

- 服务订单状态落 `zc_id_stus-service` 叶表（`stus-trade` 为中间层，子表 purchase/retail/service）。种子：ST-ORDERED/ACCEPTED/DISPATCHED/SIGNED/COMPLETED（设计 5 态）+ 辅助态（PREPARING/ACCIDENT/ARRIVED/CANCELLED/PENDING_PAYMENT/SETTLED）。
- 码映射：ST-NEW→ST-ORDERED、ST-IN_TRANSIT→ST-DISPATCHED、ST-DELIVERED→ST-SIGNED。
- **受理（委托级）先于派车**——转换凭证的发射点与状态机事件挂接必须一致，防双计（同一转换只在一个服务发射）。

## 4. fk_index 注册

凭证叶表（com/slf）须注册 forward（真实物理列）+ 全部 reverse 数组（zc_id_scale/scal-amount/scal-date/production/storage/subjects/scene/factor/function/cate-sto-title/coun-*-journal/unit-pricing），`FK_FORWARD/FK_REVERSE` 保持字典序（`binary_search_by_key` 依赖，`fk_index_keys_sorted` 测试把关）。journal 归属：smt→coun-acc、tsp→coun-ctn、whs/slf→coun-plc。

## 5. 测试模式与陷阱清单

- **受理前置 fixture**：派车校验依赖在途余额，测试须先写受理 tsp 行（或调用受理动作）；fixture 用唯一 code 前缀 + 事务回滚/清理。
- **事务可见性**：service 经 pool 独立连接，测试 fixture 须先 `tx.commit()` 再调 service（未提交行对 pool 不可见）。
- **陷阱**（均有实测教训）：
  - NULL 三值逻辑：`NOT (release = 'true' AND w > 0)` 在无 release 键时得 NULL→误判损坏；须 `NOT (COALESCE(release='true', false) AND w > 0)`。
  - 连字符列名（`ck_sto-title`/`fk_subj-storage`）SQL 中必须双引号。
  - 动态 SQL（`format!` 拼列名）触发 sqlx 注入审计 E0277——拆静态双语句。
  - `comments` 列必须合法 JSON（历史 `'货运派车记录'` 文本致 22P02）；解析侧加 `IS JSON OBJECT` 防护。
  - 继承语义：`DELETE FROM 父表` 级联删子表行；`SELECT FROM 父表` 含子表行（读聚合合法，插入不路由）。
  - 业务数据操作前 `backup-ddl.sh`；种子只落叶表。

## 6. 价目写路径（产品定价，2026-08-12 落地）

| 动作 | 表 | 写点 | 要点 |
|---|---|---|---|
| 产品单值价 | `zc_id_scal-price`（mark=单价、sk_unit=计价单位）→ 产品 `qk_price` | 产品创建/更新事务 | code `PRC-{FAMILY}-{ms}`；币种写 `sk_currency`（zc_id_unit） |
| 价目条目创建 | `zc_id_form-calculation` + `zc_id_production_r_pricing`（两行同事务） | POST `/products/{id}/pricing` | 公式行 code `CAL-PRC-{ms}`；关系行 ref_left=产品/ref_right=公式/vk_calc_pricing=公式，code `PRC-LINK-{ms}`；**API 返回/定位 id = 公式行 id** |
| 价目更新 | `zc_id_form-calculation`（UPDATE…FROM 关系表） | PUT `/pricing/{id}` | WHERE `rp.vk_calc_pricing = $1`（非 rp.id）；UPDATE FROM 双表同名列须表限定（`c.notice`） |
| 停用 | `zc_id_production_r_pricing` 软删（公式行保留） | DELETE `/pricing/{id}` | 停用后计价回退产品单值价 |
| 计价 | GET `/products/{id}/quote?qty=` | 读侧 | 优先级：活跃且生效期内条目 > 单值价 > 无价；表达式 JSON：单值 `{"kind":"unit","price":N}` / 阶梯 `{"kind":"tier","tiers":[[start,price],…]}` |

**契约细节**：
- **零 DDL**：模型原生定价族（`zc_id_formula` → form-calculation/condition/mapping；production_r_pricing → lifecycle_r_tags；unit-price → zc_id_unit）。
- `form-calculation.context` 为 jsonb：写 `jsonb_build_object('unit_id',…, 'currency_id',…)`（**禁 `::text`**——jsonb 列收 text 报类型错）；读 `(context::json->>'unit_id')::bigint`（json 提取返回 text，须 cast）。
- `exe_type` 列**已删除**（模型中心执行，2026-09-23）：平台表达式引擎统一为 Rhai（唯一引擎）⇒ 执行策略轴无消费方；`context` 固定 `{"engine":"rhai"}`。
- **NUMERIC 解码**：scal-* 的 mark 是 NUMERIC，sqlx `Option<f64>` 解码报 FLOAT8/NUMERIC 不兼容——必须 `Option<rust_decimal::Decimal>` + `ToPrimitive::to_f64()`。
- 计价单位/币种字典 `zc_id_unit` 134 行种子（吨/千克/立方米/CNY/USD…），**测试库重置后须补种**（seed 脚本 Phase 0c）。

## 7. 测试基建韧性（2026-08-12 实测）

并发会话重置测试库会丢：`zc_id_unit` 134 行、`zc_id_cate-sto-title` 4 行、`zc_id_stus-service` 11 行、`mv_inventory`（回退旧 3 列 bom 聚合定义）。**seed-wz-e2e-data.sh Phase 0c 已固化**（unit 幂等 INSERT + mv 重建为新定义 v10.1.16 容量聚合）。测试失败先查这三类种子/视图，勿先改代码。
