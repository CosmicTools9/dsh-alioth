# Ledger-Entry Service 设计笔记

> 来源：`mise run schema-info -- describe-table zc_id_docu-accounting / zc_id_stor-account / zc_id_subjects_rr_account`（2026-08-12）
> 原型：`Pre-Proc/Alioth/Prototypes/Modules/ledger-entry/m-v1.html`

---

## 一、主表字段映射表

### 1.1 LedgerEntry — zc_id_docu-accounting（归档-会计凭证）

**总列数**：27 | **基列**：12 | **业务列**：15

#### 基列（auto / semi-auto）

| 业务语义名 | 物理列          | 类型        | nullable | default         | 说明     |
| ---------- | --------------- | ----------- | -------- | --------------- | -------- |
| 创建时间   | `created_at`    | timestamptz | false    | now()           |          |
| 更新时间   | `updated_at`    | timestamptz | false    | now()           |          |
| 主键       | `id`            | bigint      | false    | gen_next_zuid() | PK       |
| 创建人ID   | `created_by_id` | bigint      | true     | —               | 系统自动 |
| 更新人ID   | `updated_by_id` | bigint      | true     | —               | 系统自动 |
| 删除时间   | `deleted_at`    | timestamptz | true     | —               | 软删     |
| 删除人ID   | `deleted_by_id` | bigint      | true     | —               |          |

#### 业务标量列（scalar）

| 业务语义名 | 物理列        | 类型   | nullable | 说明         |
| ---------- | ------------- | ------ | -------- | ------------ |
| 凭证编号   | `code`        | text   | true     | 业务编码     |
| 凭证号     | `o_number`    | text   | true     | 前台展示号   |
| 凭证备注   | `notice`      | text   | true     |              |
| 审批意见   | `comments`    | text   | true     |              |
| 场景       | `dk_scene`    | bigint | true     | dk_scene     |
| 因子       | `dk_factor`   | bigint | true     | dk_factor    |
| 功能       | `dk_function` | bigint | true     | dk_function  |
| 模板ID     | `tpl_id`      | bigint | true     |              |
| 版本号     | `tk_version`  | bigint | true     | tk_ 版本引用 |
| 批次号     | `tk_batch_no` | bigint | true     | tk_ 批次引用 |
| 前序凭证   | `fk_previous` | bigint | true     | FK自引用     |
| 分支       | `ck_branch`   | bigint | true     | ck_ 分类     |

#### 业务数组列（ARRAY）

| 业务语义名 | 物理列            | 类型  | nullable | 说明     |
| ---------- | ----------------- | ----- | -------- | -------- |
| 受益用户   | `ak_benefit_user` | ARRAY | true     | 归属用户 |
| 许可用户   | `ak_permit_user`  | ARRAY | true     | 权限用户 |
| 访问用户   | `ak_access_user`  | ARRAY | true     | 可见用户 |
| 来源       | `ak_source`       | ARRAY | true     | 渠道来源 |

#### 系统/元字段

| 业务语义名 | 物理列       | 类型 | nullable | 说明       |
| ---------- | ------------ | ---- | -------- | ---------- |
| 预测       | `projection` | text | true     |            |
| from标识   | `_f_`        | text | true     | 系统字段   |
| to标识     | `_t_`        | text | true     | 系统字段   |
| 标签色     | `t_color_`   | text | true     | UI标签颜色 |

---

### 1.2 Account — zc_id_stor-account（储元-账户）

**总列数**：28 | **基列**：12 | **业务列**：16

#### 基列（auto / semi-auto）

| 业务语义名 | 物理列          | 类型        | nullable | default         | 说明     |
| ---------- | --------------- | ----------- | -------- | --------------- | -------- |
| 创建时间   | `created_at`    | timestamptz | false    | now()           |          |
| 更新时间   | `updated_at`    | timestamptz | false    | now()           |          |
| 主键       | `id`            | bigint      | false    | gen_next_zuid() | PK       |
| 创建人ID   | `created_by_id` | bigint      | true     | —               | 系统自动 |
| 更新人ID   | `updated_by_id` | bigint      | true     | —               | 系统自动 |
| 删除时间   | `deleted_at`    | timestamptz | true     | —               | 软删     |
| 删除人ID   | `deleted_by_id` | bigint      | true     | —               |          |

#### 业务标量列（scalar）

| 业务语义名 | 物理列        | 类型   | nullable | 说明         |
| ---------- | ------------- | ------ | -------- | ------------ |
| 账户编号   | `code`        | text   | true     | 业务编码     |
| 账户号     | `o_number`    | text   | true     | 前台展示号   |
| 账户名称   | `name`        | text   | true     | 账户名称     |
| 账号       | `account`     | text   | true     | 账号         |
| 账户备注   | `notice`      | text   | true     |              |
| 审批意见   | `comments`    | text   | true     |              |
| 场景       | `dk_scene`    | bigint | true     | dk_scene     |
| 因子       | `dk_factor`   | bigint | true     | dk_factor    |
| 功能       | `dk_function` | bigint | true     | dk_function  |
| 模板ID     | `tpl_id`      | bigint | true     |              |
| 来源       | `ak_source`   | ARRAY  | true     | 渠道来源     |
| 单元       | `sk_unit`     | bigint | true     | sk_ 标准单元 |
| 托管人     | `fk_trustee`  | bigint | true     | FK 托管人    |
| 容量       | `qk_capacity` | bigint | true     | qk_ 标量引用 |

#### 业务数组列（ARRAY）

| 业务语义名 | 物理列            | 类型  | nullable | 说明     |
| ---------- | ----------------- | ----- | -------- | -------- |
| 受益用户   | `ak_benefit_user` | ARRAY | true     | 归属用户 |
| 许可用户   | `ak_permit_user`  | ARRAY | true     | 权限用户 |
| 访问用户   | `ak_access_user`  | ARRAY | true     | 可见用户 |

#### 系统/元字段

| 业务语义名 | 物理列       | 类型 | nullable | 说明       |
| ---------- | ------------ | ---- | -------- | ---------- |
| 预测       | `projection` | text | true     |            |
| from标识   | `_f_`        | text | true     | 系统字段   |
| to标识     | `_t_`        | text | true     | 系统字段   |
| 标签色     | `t_color_`   | text | true     | UI标签颜色 |

---

### 1.3 SubjectAccount — zc_id_subjects_rr_account（关联-主体↔账户）

**总列数**：14 | **基列**：8 | **业务列**：6

#### 基列（auto / semi-auto）

| 业务语义名 | 物理列          | 类型        | nullable | default                     | 说明     |
| ---------- | --------------- | ----------- | -------- | --------------------------- | -------- |
| 创建时间   | `created_at`    | timestamptz | false    | now()                       |          |
| 更新时间   | `updated_at`    | timestamptz | false    | now()                       |          |
| 主键       | `id`            | bigint      | false    | gen_next_uid((438)::bigint) | PK       |
| 创建人ID   | `created_by_id` | bigint      | true     | —                           | 系统自动 |
| 更新人ID   | `updated_by_id` | bigint      | true     | —                           | 系统自动 |
| 删除时间   | `deleted_at`    | timestamptz | true     | —                           | 软删     |
| 删除人ID   | `deleted_by_id` | bigint      | true     | —                           |          |

#### 业务标量列（scalar）

| 业务语义名 | 物理列      | 类型   | nullable | 说明             |
| ---------- | ----------- | ------ | -------- | ---------------- |
| 关系编号   | `code`      | text   | true     | 业务编码         |
| 主体ID     | `ref_left`  | bigint | true     | 关联主体（左侧） |
| 账户ID     | `ref_right` | bigint | true     | 关联账户（右侧） |
| 关系备注   | `comments`  | text   | true     |                  |
| 会计期间   | `qk_period` | bigint | true     | qk_ 标量引用     |

#### 系统/元字段

| 业务语义名 | 物理列     | 类型 | nullable | 说明       |
| ---------- | ---------- | ---- | -------- | ---------- |
| 凭证备注   | `notice`   | text | true     |            |
| 标签色     | `t_color_` | text | true     | UI标签颜色 |

---

## 二、外键关系

**outgoing_foreign_keys**: `[]`（无声明外键）
**incoming_foreign_keys**: `[]`（无表引用此主键）

> 注意：所有 `fk_*` / `ck_*` / `qk_*` / `sk_*` / `dk_*` / `tk_*` 均为 bigint，数据库层未建约束。relationships 节按 DB 实有外键（空）填写。
> SubjectAccount 的 ref_left / ref_right 为语义外键（bigint），指向其他实体主键，DB 无约束。

---

## 三、DTO 设计

### 3.1 LedgerEntry — ListItem（列表行）

返回字段（轻量）：

| DTO字段       | 映射列        | 类型         |
| ------------- | ------------- | ------------ |
| `id`          | `id`          | i64          |
| `code`        | `code`        | String       |
| `o_number`    | `o_number`    | String       |
| `tk_version`  | `tk_version`  | i64 via _ref |
| `tk_batch_no` | `tk_batch_no` | i64 via _ref |
| `created_at`  | `created_at`  | DateTime     |
| `dk_scene`    | `dk_scene`    | i64          |

### 3.2 LedgerEntry — Detail（详情）

| DTO字段           | 映射列            | 类型         |
| ----------------- | ----------------- | ------------ |
| `id`              | `id`              | i64          |
| `code`            | `code`            | String       |
| `o_number`        | `o_number`        | String       |
| `notice`          | `notice`          | String       |
| `t_color_`        | `t_color_`        | String       |
| `comments`        | `comments`        | String       |
| `projection`      | `projection`      | String       |
| `ak_benefit_user` | `ak_benefit_user` | Vec<i64>     |
| `ak_permit_user`  | `ak_permit_user`  | Vec<i64>     |
| `ak_access_user`  | `ak_access_user`  | Vec<i64>     |
| `ak_source`       | `ak_source`       | Vec<i64>     |
| `tpl_id`          | `tpl_id`          | i64          |
| `tk_version`      | `tk_version`      | i64 via _ref |
| `tk_batch_no`     | `tk_batch_no`     | i64 via _ref |
| `fk_previous`     | `fk_previous`     | i64 via _ref |
| `ck_branch`       | `ck_branch`       | i64 via _ref |
| `dk_scene`        | `dk_scene`        | i64          |
| `dk_factor`       | `dk_factor`       | i64          |
| `dk_function`     | `dk_function`     | i64          |
| `_f_`             | `_f_`             | String       |
| `_t_`             | `_t_`             | String       |
| `created_at`      | `created_at`      | DateTime     |
| `updated_at`      | `updated_at`      | DateTime     |
| `created_by_id`   | `created_by_id`   | i64          |
| `updated_by_id`   | `updated_by_id`   | i64          |

### 3.3 LedgerEntry — Create（创建）

| DTO字段           | 映射列            | 类型     |
| ----------------- | ----------------- | -------- |
| `code`            | `code`            | String   |
| `o_number`        | `o_number`        | String   |
| `notice`          | `notice`          | String   |
| `t_color_`        | `t_color_`        | String   |
| `comments`        | `comments`        | String   |
| `projection`      | `projection`      | String   |
| `ak_benefit_user` | `ak_benefit_user` | Vec<i64> |
| `ak_permit_user`  | `ak_permit_user`  | Vec<i64> |
| `ak_access_user`  | `ak_access_user`  | Vec<i64> |
| `ak_source`       | `ak_source`       | Vec<i64> |
| `tpl_id`          | `tpl_id`          | i64      |
| `tk_version`      | `tk_version`      | i64      |
| `tk_batch_no`     | `tk_batch_no`     | i64      |
| `ck_branch`       | `ck_branch`       | i64      |
| `dk_scene`        | `dk_scene`        | i64      |
| `dk_factor`       | `dk_factor`       | i64      |
| `dk_function`     | `dk_function`     | i64      |

### 3.4 LedgerEntry — Update（更新）

同 Create，额外含 `id` 字段。

### 3.5 Account — ListItem（列表行）

| DTO字段      | 映射列       | 类型         |
| ------------ | ------------ | ------------ |
| `id`         | `id`         | i64          |
| `code`       | `code`       | String       |
| `name`       | `name`       | String       |
| `account`    | `account`    | String       |
| `sk_unit`    | `sk_unit`    | i64 via _ref |
| `created_at` | `created_at` | DateTime     |

### 3.6 Account — Detail（详情）

| DTO字段           | 映射列            | 类型         |
| ----------------- | ----------------- | ------------ |
| `id`              | `id`              | i64          |
| `code`            | `code`            | String       |
| `o_number`        | `o_number`        | String       |
| `name`            | `name`            | String       |
| `account`         | `account`         | String       |
| `notice`          | `notice`          | String       |
| `t_color_`        | `t_color_`        | String       |
| `comments`        | `comments`        | String       |
| `projection`      | `projection`      | String       |
| `ak_benefit_user` | `ak_benefit_user` | Vec<i64>     |
| `ak_permit_user`  | `ak_permit_user`  | Vec<i64>     |
| `ak_access_user`  | `ak_access_user`  | Vec<i64>     |
| `ak_source`       | `ak_source`       | Vec<i64>     |
| `tpl_id`          | `tpl_id`          | i64          |
| `sk_unit`         | `sk_unit`         | i64 via _ref |
| `fk_trustee`      | `fk_trustee`      | i64 via _ref |
| `qk_capacity`     | `qk_capacity`     | i64 via _ref |
| `dk_scene`        | `dk_scene`        | i64          |
| `dk_factor`       | `dk_factor`       | i64          |
| `dk_function`     | `dk_function`     | i64          |
| `_f_`             | `_f_`             | String       |
| `_t_`             | `_t_`             | String       |
| `created_at`      | `created_at`      | DateTime     |
| `updated_at`      | `updated_at`      | DateTime     |
| `created_by_id`   | `created_by_id`   | i64          |
| `updated_by_id`   | `updated_by_id`   | i64          |

### 3.7 Account — Create（创建）

| DTO字段           | 映射列            | 类型     |
| ----------------- | ----------------- | -------- |
| `code`            | `code`            | String   |
| `o_number`        | `o_number`        | String   |
| `name`            | `name`            | String   |
| `account`         | `account`         | String   |
| `notice`          | `notice`          | String   |
| `t_color_`        | `t_color_`        | String   |
| `comments`        | `comments`        | String   |
| `projection`      | `projection`      | String   |
| `ak_benefit_user` | `ak_benefit_user` | Vec<i64> |
| `ak_permit_user`  | `ak_permit_user`  | Vec<i64> |
| `ak_access_user`  | `ak_access_user`  | Vec<i64> |
| `ak_source`       | `ak_source`       | Vec<i64> |
| `tpl_id`          | `tpl_id`          | i64      |
| `sk_unit`         | `sk_unit`         | i64      |
| `fk_trustee`      | `fk_trustee`      | i64      |
| `qk_capacity`     | `qk_capacity`     | i64      |
| `dk_scene`        | `dk_scene`        | i64      |
| `dk_factor`       | `dk_factor`       | i64      |
| `dk_function`     | `dk_function`     | i64      |

### 3.8 Account — Update（更新）

同 Create，额外含 `id` 字段。

### 3.9 SubjectAccount — ListItem（列表行）

| DTO字段      | 映射列       | 类型         |
| ------------ | ------------ | ------------ |
| `id`         | `id`         | i64          |
| `code`       | `code`       | String       |
| `ref_left`   | `ref_left`   | i64 via _ref |
| `ref_right`  | `ref_right`  | i64 via _ref |
| `qk_period`  | `qk_period`  | i64 via _ref |
| `created_at` | `created_at` | DateTime     |

### 3.10 SubjectAccount — Detail（详情）

| DTO字段         | 映射列          | 类型         |
| --------------- | --------------- | ------------ |
| `id`            | `id`            | i64          |
| `code`          | `code`          | String       |
| `ref_left`      | `ref_left`      | i64 via _ref |
| `ref_right`     | `ref_right`     | i64 via _ref |
| `comments`      | `comments`      | String       |
| `qk_period`     | `qk_period`     | i64 via _ref |
| `notice`        | `notice`        | String       |
| `t_color_`      | `t_color_`      | String       |
| `created_at`    | `created_at`    | DateTime     |
| `updated_at`    | `updated_at`    | DateTime     |
| `created_by_id` | `created_by_id` | i64          |
| `updated_by_id` | `updated_by_id` | i64          |

### 3.11 SubjectAccount — Create（创建）

| DTO字段     | 映射列      | 类型   |
| ----------- | ----------- | ------ |
| `code`      | `code`      | String |
| `ref_left`  | `ref_left`  | i64    |
| `ref_right` | `ref_right` | i64    |
| `comments`  | `comments`  | String |
| `qk_period` | `qk_period` | i64    |
| `notice`    | `notice`    | String |
| `t_color_`  | `t_color_`  | String |

### 3.12 SubjectAccount — Update（更新）

同 Create，额外含 `id` 字段。

---

## 四、幻影风险点

| 风险 | 说明                                                                                                                                    |
| ---- | --------------------------------------------------------------------------------------------------------------------------------------- |
| 无   | DB无声明外键；所有 `fk_*` / `ck_*` / `qk_*` / `sk_*` / `dk_*` / `tk_*` / `ref_*` 均为 bigint，service.json 按 DB 原始类型映射，无幻影。 |

> 全部 69 列（27 + 28 + 14）均经 `information_schema.columns` 验证。
