# warehouse Service 设计笔记

> 来源：`mise run schema-info -- describe-table zc_id_cate-warehouse` + `describe-table zc_id_stor-plc-warehouse`（2026-08-12）
> 原型：`Pre-Proc/Alioth/Prototypes/Modules/warehouse/m-v3.html`
> 语义映射：capability-map.md CAP-WH-001（仓库主数据管理）+ CAP-WH-002（库区库位管理）

---

## 一、实体说明

### 1.1 Warehouse（仓库类目）→ `isahl.zc_id_cate-warehouse`

**业务语义**：类目-仓储类型。仓库类别主数据，记录仓库分类信息，含编码、启用状态、排序。

原型语义（capability-map CAP-WH-001）：

- 仓库列表 → code（仓库编码）、enable（启用/停用）、c_sort_（排序）
- 仓库名称映射至 `code`（业务编码）
- `o_number` → 前台展示编号

### 1.2 WarehouseLocation（仓位/库位）→ `isahl.zc_id_stor-plc-warehouse`

**业务语义**：场所-仓库。仓库库位（仓位）主数据，记录具体存放位置。

原型语义（capability-map CAP-WH-002）：

- 库位编码 → code
- 所属仓库 → fk_warehouse（关联 Warehouse.id）
- 区域/货架/层位 → sk_unit（标准库位标识）
- 库位状态 → 通过 capacity/max_capacity 推算
- 所属地址 → fk_address（地址外键）
- 托管方 → fk_trustee（受托方）
- 库位类型/属性 → _f_/_t_（系统字段）或 ck_category

---

## 二、主表字段映射表

### 2.1 Warehouse（`isahl.zc_id_cate-warehouse`）

**总列数**：17 | **基列**（auto）：7 | **业务列**：10

#### 基列（auto）

| 业务语义名 | 物理列          | 类型        | nullable | default          | 说明       |
| ---------- | --------------- | ----------- | -------- | ---------------- | ---------- |
| 创建时间   | `created_at`    | timestamptz | false    | now()            |            |
| 更新时间   | `updated_at`    | timestamptz | false    | now()            |            |
| 主键       | `id`            | bigint      | false    | gen_next_uid(50) | PK，序列50 |
| 创建人ID   | `created_by_id` | bigint      | true     | —                | 系统自动   |
| 更新人ID   | `updated_by_id` | bigint      | true     | —                | 系统自动   |
| 删除时间   | `deleted_at`    | timestamptz | true     | —                | 软删       |
| 删除人ID   | `deleted_by_id` | bigint      | true     | —                |            |

#### 业务标量列（scalar）

| 业务语义名 | 物理列     | 类型   | nullable | 说明       |
| ---------- | ---------- | ------ | -------- | ---------- |
| 备注       | `notice`   | text   | true     |            |
| 标签色     | `t_color_` | text   | true     | UI标签颜色 |
| 仓库编码   | `code`     | text   | true     | 业务编码   |
| 展示编号   | `o_number` | text   | true     | 前台展示号 |
| 审批意见   | `comments` | text   | true     |            |
| 排序号     | `c_sort_`  | bigint | true     | 列表排序   |

#### 业务数组列（ARRAY）

| 业务语义名 | 物理列            | 类型  | nullable | 说明     |
| ---------- | ----------------- | ----- | -------- | -------- |
| 受益用户   | `ak_benefit_user` | ARRAY | true     | 归属用户 |
| 许可用户   | `ak_permit_user`  | ARRAY | true     | 权限用户 |
| 访问用户   | `ak_access_user`  | ARRAY | true     | 可见用户 |

#### 业务布尔列

| 业务语义名 | 物理列   | 类型    | nullable | 说明          |
| ---------- | -------- | ------- | -------- | ------------- |
| 启用       | `enable` | boolean | true     | 仓库启用/停用 |

---

### 2.2 WarehouseLocation（`isahl.zc_id_stor-plc-warehouse`）

**总列数**：29 | **基列**（auto）：7 | **业务列**：22

#### 基列（auto）

| 业务语义名 | 物理列          | 类型        | nullable | default         | 说明         |
| ---------- | --------------- | ----------- | -------- | --------------- | ------------ |
| 创建时间   | `created_at`    | timestamptz | false    | now()           |              |
| 更新时间   | `updated_at`    | timestamptz | false    | now()           |              |
| 主键       | `id`            | bigint      | false    | gen_next_zuid() | PK，zuid序列 |
| 创建人ID   | `created_by_id` | bigint      | true     | —               | 系统自动     |
| 更新人ID   | `updated_by_id` | bigint      | true     | —               | 系统自动     |
| 删除时间   | `deleted_at`    | timestamptz | true     | —               | 软删         |
| 删除人ID   | `deleted_by_id` | bigint      | true     | —               |              |

#### 业务标量列（scalar）

| 业务语义名 | 物理列         | 类型   | nullable | 说明                        |
| ---------- | -------------- | ------ | -------- | --------------------------- |
| 备注       | `notice`       | text   | true     |                             |
| 标签色     | `t_color_`     | text   | true     | UI标签颜色                  |
| 库位编码   | `code`         | text   | true     | 业务编码                    |
| 展示编号   | `o_number`     | text   | true     | 前台展示号                  |
| 审批意见   | `comments`     | text   | true     |                             |
| 库位标识   | `projection`   | text   | true     | 库位投影/编码               |
| from标识   | `_f_`          | text   | true     | 系统字段                    |
| to标识     | `_t_`          | text   | true     | 系统字段                    |
| 场景       | `dk_scene`     | bigint | true     | dk_scene                    |
| 因子       | `dk_factor`    | bigint | true     | dk_factor                   |
| 功能       | `dk_function`  | bigint | true     | dk_function                 |
| 模板ID     | `tpl_id`       | bigint | true     |                             |
| 所属仓库   | `fk_warehouse` | bigint | true     | FK→Warehouse.id（语义外键） |
| 地址外键   | `fk_address`   | bigint | true     | FK，地址表                  |
| 库位属性   | `sk_unit`      | bigint | true     | sk_标准库位                 |
| 托管方     | `fk_trustee`   | bigint | true     | FK，受托方                  |
| 库位分类   | `ck_category`  | bigint | true     | ck_分类                     |
| 状态       | `ck_status`    | bigint | true     | ck_状态                     |
| 排序号     | `c_sort_`      | bigint | true     | 排序                        |

#### 业务数组列（ARRAY）

| 业务语义名 | 物理列            | 类型  | nullable | 说明     |
| ---------- | ----------------- | ----- | -------- | -------- |
| 受益用户   | `ak_benefit_user` | ARRAY | true     | 归属用户 |
| 许可用户   | `ak_permit_user`  | ARRAY | true     | 权限用户 |
| 访问用户   | `ak_access_user`  | ARRAY | true     | 可见用户 |
| 来源       | `ak_source`       | ARRAY | true     | 渠道来源 |

#### 业务布尔列

| 业务语义名 | 物理列    | 类型    | nullable | 说明         |
| ---------- | --------- | ------- | -------- | ------------ |
| 是否完成   | `is_done` | boolean | true     | 库位完成状态 |

---

## 三、外键关系

**outgoing_foreign_keys**: `[]`（无声明外键）
**incoming_foreign_keys**: `[]`（无表引用此主键）

> 注意：fk_warehouse / fk_address / fk_trustee 为语义外键（bigint），但数据库层未建约束。relationships 节按 DB 实有外键（空）填写。

---

## 四、DTO 设计

### 4.1 Warehouse（仓库类目）

#### ListItem（列表行）

| DTO字段      | 映射列       | 类型     |
| ------------ | ------------ | -------- |
| `id`         | `id`         | i64      |
| `code`       | `code`       | String   |
| `o_number`   | `o_number`   | String   |
| `enable`     | `enable`     | bool     |
| `c_sort_`    | `c_sort_`    | i64      |
| `created_at` | `created_at` | DateTime |

#### Detail（详情）

| DTO字段           | 映射列            | 类型     |
| ----------------- | ----------------- | -------- |
| `id`              | `id`              | i64      |
| `code`            | `code`            | String   |
| `o_number`        | `o_number`        | String   |
| `notice`          | `notice`          | String   |
| `t_color_`        | `t_color_`        | String   |
| `comments`        | `comments`        | String   |
| `ak_benefit_user` | `ak_benefit_user` | Vec<i64> |
| `ak_permit_user`  | `ak_permit_user`  | Vec<i64> |
| `ak_access_user`  | `ak_access_user`  | Vec<i64> |
| `enable`          | `enable`          | bool     |
| `c_sort_`         | `c_sort_`         | i64      |
| `created_at`      | `created_at`      | DateTime |
| `updated_at`      | `updated_at`      | DateTime |
| `created_by_id`   | `created_by_id`   | i64      |
| `updated_by_id`   | `updated_by_id`   | i64      |

#### Create（创建）

| DTO字段           | 映射列            | 类型     |
| ----------------- | ----------------- | -------- |
| `code`            | `code`            | String   |
| `o_number`        | `o_number`        | String   |
| `notice`          | `notice`          | String   |
| `t_color_`        | `t_color_`        | String   |
| `comments`        | `comments`        | String   |
| `ak_benefit_user` | `ak_benefit_user` | Vec<i64> |
| `ak_permit_user`  | `ak_permit_user`  | Vec<i64> |
| `ak_access_user`  | `ak_access_user`  | Vec<i64> |
| `enable`          | `enable`          | bool     |
| `c_sort_`         | `c_sort_`         | i64      |

#### Update（更新）

同 Create，额外含 `id` 字段。

---

### 4.2 WarehouseLocation（库位/仓位）

#### ListItem（列表行）

| DTO字段        | 映射列         | 类型     |
| -------------- | -------------- | -------- |
| `id`           | `id`           | i64      |
| `code`         | `code`         | String   |
| `o_number`     | `o_number`     | String   |
| `fk_warehouse` | `fk_warehouse` | i64      |
| `ck_status`    | `ck_status`    | i64      |
| `c_sort_`      | `c_sort_`      | i64      |
| `is_done`      | `is_done`      | bool     |
| `created_at`   | `created_at`   | DateTime |
| `dk_scene`     | `dk_scene`     | i64      |

#### Detail（详情）

| DTO字段           | 映射列            | 类型     |
| ----------------- | ----------------- | -------- |
| `id`              | `id`              | i64      |
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
| `fk_warehouse`    | `fk_warehouse`    | i64      |
| `fk_address`      | `fk_address`      | i64      |
| `sk_unit`         | `sk_unit`         | i64      |
| `fk_trustee`      | `fk_trustee`      | i64      |
| `ck_category`     | `ck_category`     | i64      |
| `ck_status`       | `ck_status`       | i64      |
| `is_done`         | `is_done`         | bool     |
| `c_sort_`         | `c_sort_`         | i64      |
| `dk_scene`        | `dk_scene`        | i64      |
| `dk_factor`       | `dk_factor`       | i64      |
| `dk_function`     | `dk_function`     | i64      |
| `_f_`             | `_f_`             | String   |
| `_t_`             | `_t_`             | String   |
| `created_at`      | `created_at`      | DateTime |
| `updated_at`      | `updated_at`      | DateTime |
| `created_by_id`   | `created_by_id`   | i64      |
| `updated_by_id`   | `updated_by_id`   | i64      |

#### Create（创建）

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
| `fk_warehouse`    | `fk_warehouse`    | i64      |
| `fk_address`      | `fk_address`      | i64      |
| `sk_unit`         | `sk_unit`         | i64      |
| `fk_trustee`      | `fk_trustee`      | i64      |
| `ck_category`     | `ck_category`     | i64      |
| `ck_status`       | `ck_status`       | i64      |
| `is_done`         | `is_done`         | bool     |
| `c_sort_`         | `c_sort_`         | i64      |
| `dk_scene`        | `dk_scene`        | i64      |
| `dk_factor`       | `dk_factor`       | i64      |
| `dk_function`     | `dk_function`     | i64      |

#### Update（更新）

同 Create，额外含 `id` 字段。

---

## 五、幻影风险点

| 风险 | 说明                                                                                                                                                    |
| ---- | ------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 无   | DB无声明外键；所有 `fk_*` / `ck_*` / `dk_*` / `sk_*` 均为 bigint，service.json 按 DB 原始类型映射，无幻影。所有列均取自 `describe-table` 输出，零容忍。 |

---

## 六、关系

- Warehouse（仓库类目）：一对多→ WarehouseLocation（库位）。通过 WarehouseLocation.fk_warehouse → Warehouse.id 关联（语义外键，无 DB 约束）。
