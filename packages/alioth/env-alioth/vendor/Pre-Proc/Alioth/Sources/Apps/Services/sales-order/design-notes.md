# Sales-Order Service 设计笔记

> 来源：`mise run schema-info -- describe-table zc_id_oper-sales_order`（2026-08-12）
> 原型：`Pre-Proc/Alioth/Prototypes/Modules/sales-order/m-v2.html`

---

## 一、主表字段映射表

**主表**：`isahl.zc_id_oper-sales_order`（操作-销售接单）
**总列数**：40 | **基列**（auto/semi-auto）：12 | **业务列**：28

### 1.1 基列（auto / semi-auto）

| 业务语义名 | 物理列          | 类型        | nullable | default         | 说明     |
| ---------- | --------------- | ----------- | -------- | --------------- | -------- |
| 创建时间   | `created_at`    | timestamptz | false    | now()           |          |
| 更新时间   | `updated_at`    | timestamptz | false    | now()           |          |
| 主键       | `id`            | bigint      | false    | gen_next_zuid() | PK       |
| 创建人ID   | `created_by_id` | bigint      | true     | —               | 系统自动 |
| 更新人ID   | `updated_by_id` | bigint      | true     | —               | 系统自动 |
| 删除时间   | `deleted_at`    | timestamptz | true     | —               | 软删     |
| 删除人ID   | `deleted_by_id` | bigint      | true     | —               |          |

### 1.2 业务标量列（scalar）

| 业务语义名 | 物理列             | 类型   | nullable | 说明         |
| ---------- | ------------------ | ------ | -------- | ------------ |
| 备注       | `notice`           | text   | true     |              |
| 标签色     | `t_color_`         | text   | true     | UI标签颜色   |
| 订单编号   | `code`             | text   | true     | 业务编码     |
| 订单号     | `o_number`         | text   | true     | 前台展示号   |
| 审批意见   | `comments`         | text   | true     |              |
| 场景       | `dk_scene`         | bigint | true     | dk_scene     |
| 因子       | `dk_factor`        | bigint | true     | dk_factor    |
| 功能       | `dk_function`      | bigint | true     | dk_function  |
| 模板ID     | `tpl_id`           | bigint | true     |              |
| 版本号     | `tk_version`       | bigint | true     | qk_ 引用     |
| 批次号     | `tk_batch_no`      | bigint | true     | qk_ 引用     |
| 前序订单   | `fk_previous`      | bigint | true     | FK自引用     |
| 分支       | `ck_branch`        | bigint | true     | ck_ 分类     |
| 工时       | `qk_work_duration` | bigint | true     | qk_ 标量引用 |
| 操作员     | `fk_operator`      | bigint | true     | FK           |
| 科目       | `fk_subject`       | bigint | true     | FK           |
| 工序号     | `op_number`        | text   | true     |              |
| 周期       | `qk_period`        | bigint | true     | qk_ 标量引用 |
| 仓库分类   | `ck_cate-wh`       | bigint | true     | ck_ 分类     |
| 工作单元   | `sk_unit-working`  | bigint | true     | sk_ 标准     |
| 到货数量   | `qk_arrived`       | bigint | true     | qk_ 标量引用 |
| 业务分类   | `ck_cate-biz`      | bigint | true     | ck_ 分类     |
| 审批人     | `fk_approve`       | bigint | true     | FK           |
| SLA        | `qk_sla`           | bigint | true     | qk_ 标量引用 |
| 紧急标识   | `lk_urgent`        | bigint | true     | lk_ lookup   |
| 工序分类   | `ck_cate-proc_op`  | bigint | true     | ck_ 分类     |

### 1.3 业务数组列（ARRAY）

| 业务语义名 | 物理列            | 类型  | nullable | 说明     |
| ---------- | ----------------- | ----- | -------- | -------- |
| 受益用户   | `ak_benefit_user` | ARRAY | true     | 归属用户 |
| 许可用户   | `ak_permit_user`  | ARRAY | true     | 权限用户 |
| 访问用户   | `ak_access_user`  | ARRAY | true     | 可见用户 |
| 来源       | `ak_source`       | ARRAY | true     | 渠道来源 |

### 1.4 系统/元字段

| 业务语义名 | 物理列       | 类型 | nullable | 说明     |
| ---------- | ------------ | ---- | -------- | -------- |
| 预测       | `projection` | text | true     |          |
| from标识   | `_f_`        | text | true     | 系统字段 |
| to标识     | `_t_`        | text | true     | 系统字段 |

---

## 二、外键关系

**outgoing_foreign_keys**: `[]`（无声明外键）
**incoming_foreign_keys**: `[]`（无表引用此主键）

> 注意：fk_operator / fk_subject / fk_approve / fk_previous 为语义外键（bigint），但数据库层未建约束。relationships 节按 DB 实有外键（空）填写。

---

## 三、DTO 设计

### 3.1 ListItem（列表行）

返回字段（轻量）：

| DTO字段       | 映射列             | 类型         |
| ------------- | ------------------ | ------------ |
| `id`          | `id`               | i64          |
| `code`        | `code`             | String       |
| `o_number`    | `o_number`         | String       |
| `op_number`   | `op_number`        | String       |
| `tk_version`  | `tk_version` (qk)  | i64 via _ref |
| `tk_batch_no` | `tk_batch_no` (qk) | i64 via _ref |
| `lk_urgent`   | `lk_urgent`        | i64          |
| `created_at`  | `created_at`       | DateTime     |
| `dk_scene`    | `dk_scene`         | i64          |

### 3.2 Detail（详情）

返回字段（完整，非auto全部）：

| DTO字段            | 映射列             | 类型         |
| ------------------ | ------------------ | ------------ |
| `id`               | `id`               | i64          |
| `code`             | `code`             | String       |
| `o_number`         | `o_number`         | String       |
| `op_number`        | `op_number`        | String       |
| `notice`           | `notice`           | String       |
| `t_color_`         | `t_color_`         | String       |
| `comments`         | `comments`         | String       |
| `projection`       | `projection`       | String       |
| `ak_benefit_user`  | `ak_benefit_user`  | Vec<i64>     |
| `ak_permit_user`   | `ak_permit_user`   | Vec<i64>     |
| `ak_access_user`   | `ak_access_user`   | Vec<i64>     |
| `ak_source`        | `ak_source`        | Vec<i64>     |
| `tpl_id`           | `tpl_id`           | i64          |
| `tk_version`       | `tk_version`       | i64 via _ref |
| `tk_batch_no`      | `tk_batch_no`      | i64 via _ref |
| `fk_previous`      | `fk_previous`      | i64 via _ref |
| `ck_branch`        | `ck_branch`        | i64 via _ref |
| `qk_work_duration` | `qk_work_duration` | i64 via _ref |
| `fk_operator`      | `fk_operator`      | i64 via _ref |
| `fk_subject`       | `fk_subject`       | i64 via _ref |
| `qk_period`        | `qk_period`        | i64 via _ref |
| `ck_cate-wh`       | `ck_cate-wh`       | i64 via _ref |
| `sk_unit-working`  | `sk_unit-working`  | i64 via _ref |
| `qk_arrived`       | `qk_arrived`       | i64 via _ref |
| `ck_cate-biz`      | `ck_cate-biz`      | i64 via _ref |
| `fk_approve`       | `fk_approve`       | i64 via _ref |
| `qk_sla`           | `qk_sla`           | i64 via _ref |
| `lk_urgent`        | `lk_urgent`        | i64          |
| `ck_cate-proc_op`  | `ck_cate-proc_op`  | i64 via _ref |
| `dk_scene`         | `dk_scene`         | i64          |
| `dk_factor`        | `dk_factor`        | i64          |
| `dk_function`      | `dk_function`      | i64          |
| `_f_`              | `_f_`              | String       |
| `_t_`              | `_t_`              | String       |
| `created_at`       | `created_at`       | DateTime     |
| `updated_at`       | `updated_at`       | DateTime     |
| `created_by_id`    | `created_by_id`    | i64          |
| `updated_by_id`    | `updated_by_id`    | i64          |

### 3.3 Create（创建）

入参字段（可写业务字段）：

| DTO字段            | 映射列             | 类型     |
| ------------------ | ------------------ | -------- |
| `code`             | `code`             | String   |
| `o_number`         | `o_number`         | String   |
| `op_number`        | `op_number`        | String   |
| `notice`           | `notice`           | String   |
| `t_color_`         | `t_color_`         | String   |
| `comments`         | `comments`         | String   |
| `projection`       | `projection`       | String   |
| `ak_benefit_user`  | `ak_benefit_user`  | Vec<i64> |
| `ak_permit_user`   | `ak_permit_user`   | Vec<i64> |
| `ak_access_user`   | `ak_access_user`   | Vec<i64> |
| `ak_source`        | `ak_source`        | Vec<i64> |
| `tpl_id`           | `tpl_id`           | i64      |
| `tk_version`       | `tk_version`       | i64      |
| `tk_batch_no`      | `tk_batch_no`      | i64      |
| `ck_branch`        | `ck_branch`        | i64      |
| `qk_work_duration` | `qk_work_duration` | i64      |
| `fk_operator`      | `fk_operator`      | i64      |
| `fk_subject`       | `fk_subject`       | i64      |
| `qk_period`        | `qk_period`        | i64      |
| `ck_cate-wh`       | `ck_cate-wh`       | i64      |
| `sk_unit-working`  | `sk_unit-working`  | i64      |
| `qk_arrived`       | `qk_arrived`       | i64      |
| `ck_cate-biz`      | `ck_cate-biz`      | i64      |
| `fk_approve`       | `fk_approve`       | i64      |
| `qk_sla`           | `qk_sla`           | i64      |
| `lk_urgent`        | `lk_urgent`        | i64      |
| `ck_cate-proc_op`  | `ck_cate-proc_op`  | i64      |
| `dk_scene`         | `dk_scene`         | i64      |
| `dk_factor`        | `dk_factor`        | i64      |
| `dk_function`      | `dk_function`      | i64      |

### 3.4 Update（更新）

入参字段（Create 超集 + id）：

同 Create，额外含 `id` 字段。

---

## 四、幻影风险点

| 风险 | 说明                                                                                                                                   |
| ---- | -------------------------------------------------------------------------------------------------------------------------------------- |
| 无   | DB无声明外键；所有 `fk_*` / `ck_*` / `qk_*` / `sk_*` / `lk_*` / `dk_*` / `tk_*` 均为 bigint，service.json 按 DB 原始类型映射，无幻影。 |

> 参照 a5c591e04 前车之鉴：Place.fk_address 幻影列问题——本次所有引用列均直接取自 `information_schema.columns`，零容忍。
