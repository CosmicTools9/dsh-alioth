# inbound-order Service 设计笔记

## 1. DB Schema

### 1.1 zc_id_plan-inbound（入库订单主表）

- **biz_name**: 计划-入库（InboundOrder）
- **schema**: isahl
- **列数**: 32

### 1.2 zc_id_bom-inbound（BOM-入库单/入库明细表）

- **biz_name**: BOM-入库单（InboundBom）
- **schema**: isahl
- **列数**: 30

### 1.3 关系

- DB 无声明外键（outgoing_foreign_keys: [], incoming_foreign_keys: []）
- 两表通过 `fk_previous`（bom.parent_id / 主表ID）建立事实父子关系
- 本 Service 建模为 `InboundOrder 1 → N InboundBom`（has_many）

---

## 2. 字段映射

### 2.1 InboundOrder（zc_id_plan-inbound）

| #   | DB 列名         | business_name   | data_type   | nullable | category | 原型语义                 |
| --- | --------------- | --------------- | ----------- | -------- | -------- | ------------------------ |
| 1   | created_at      | created_at      | timestamptz | false    | auto     | createdAt                |
| 2   | updated_at      | updated_at      | timestamptz | false    | auto     | updatedAt                |
| 3   | id              | id              | bigint      | false    | auto     | id                       |
| 4   | created_by_id   | created_by_id   | bigint      | true     | auto     | createdBy                |
| 5   | updated_by_id   | updated_by_id   | bigint      | true     | auto     | updatedBy                |
| 6   | deleted_at      | deleted_at      | timestamptz | true     | auto     | -                        |
| 7   | deleted_by_id   | deleted_by_id   | bigint      | true     | auto     | -                        |
| 8   | notice          | notice          | text        | true     | scalar   | note（备注）             |
| 9   | t_color_        | t_color_        | text        | true     | scalar   | -                        |
| 10  | code            | order_no        | text        | true     | scalar   | orderNo（入库单号）      |
| 11  | o_number        | o_number        | text        | true     | auto     | -                        |
| 12  | comments        | comments        | text        | true     | scalar   | -                        |
| 13  | ak_benefit_user | ak_benefit_user | ARRAY       | true     | scalar   | -                        |
| 14  | ak_permit_user  | ak_permit_user  | ARRAY       | true     | scalar   | -                        |
| 15  | ak_access_user  | ak_access_user  | ARRAY       | true     | scalar   | -                        |
| 16  | projection      | projection      | text        | true     | auto     | -                        |
| 17  | _f_             | _f_             | text        | true     | auto     | -                        |
| 18  | _t_             | _t_             | text        | true     | auto     | -                        |
| 19  | dk_scene        | dk_scene        | bigint      | true     | auto     | -                        |
| 20  | dk_factor       | dk_factor       | bigint      | true     | auto     | -                        |
| 21  | dk_function     | dk_function     | bigint      | true     | auto     | -                        |
| 22  | tpl_id          | tpl_id          | bigint      | true     | scalar   | tplId（模板ID）          |
| 23  | ak_source       | ak_source       | ARRAY       | true     | scalar   | tags（标签，ARRAY text） |
| 24  | cron            | cron            | text        | true     | scalar   | -                        |
| 25  | exclude         | exclude         | jsonb       | true     | scalar   | -                        |
| 26  | sort            | sort            | bigint      | true     | scalar   | -                        |
| 27  | qk_date-segm    | qk_date_segm    | bigint      | true     | scalar   | 计划日期分段             |
| 28  | qk_time-segm    | qk_time_segm    | bigint      | true     | scalar   | 计划时间分段             |
| 29  | qk_progress     | qk_progress     | bigint      | true     | scalar   | 进度状态                 |
| 30  | progress_pct    | progress_pct    | numeric     | true     | scalar   | 完成百分比               |
| 31  | schedule_pct    | schedule_pct    | numeric     | true     | scalar   | 排期百分比               |
| 32  | lk_health       | lk_health       | bigint      | true     | scalar   | 健康度/优先级            |

**原型语义补充**（capability-map.md）：

- `orderNo` = code（入库单号，如 PO-20260811-001）
- `supplierId` → 无对应列，属主数据外键引用
- `supplierName` → 无对应列，属主数据外键引用
- `warehouseCode` → 无对应列，属仓库主数据外键引用
- `channel` = type（DIRECT 直送 / TRANSFER 调拨）
- `expectedArrivalDate` → 无对应列，属外接主数据
- `status` = qk_progress（进度状态）→ 本 Service 映射到 qk_progress
- `priority` = lk_health（优先级）
- `items` = has_many → InboundBom

### 2.2 InboundBom（zc_id_bom-inbound）

| #   | DB 列名         | business_name   | data_type   | nullable | category | 原型语义                    |
| --- | --------------- | --------------- | ----------- | -------- | -------- | --------------------------- |
| 1   | created_at      | created_at      | timestamptz | false    | auto     | -                           |
| 2   | updated_at      | updated_at      | timestamptz | false    | auto     | -                           |
| 3   | id              | id              | bigint      | false    | auto     | 行ID                        |
| 4   | created_by_id   | created_by_id   | bigint      | true     | auto     | -                           |
| 5   | updated_by_id   | updated_by_id   | bigint      | true     | auto     | -                           |
| 6   | deleted_at      | deleted_at      | timestamptz | true     | auto     | -                           |
| 7   | deleted_by_id   | deleted_by_id   | bigint      | true     | auto     | -                           |
| 8   | notice          | notice          | text        | true     | scalar   | -                           |
| 9   | t_color_        | t_color_        | text        | true     | scalar   | -                           |
| 10  | code            | code            | text        | true     | scalar   | 行编号                      |
| 11  | o_number        | o_number        | text        | true     | auto     | -                           |
| 12  | comments        | comments        | text        | true     | scalar   | -                           |
| 13  | ak_benefit_user | ak_benefit_user | ARRAY       | true     | scalar   | -                           |
| 14  | ak_permit_user  | ak_permit_user  | ARRAY       | true     | scalar   | -                           |
| 15  | ak_access_user  | ak_access_user  | ARRAY       | true     | scalar   | -                           |
| 16  | projection      | projection      | text        | true     | auto     | -                           |
| 17  | _f_             | _f_             | text        | true     | auto     | -                           |
| 18  | _t_             | _t_             | text        | true     | auto     | -                           |
| 19  | dk_scene        | dk_scene        | bigint      | true     | auto     | -                           |
| 20  | dk_factor       | dk_factor       | bigint      | true     | auto     | -                           |
| 21  | dk_function     | dk_function     | bigint      | true     | auto     | -                           |
| 22  | tpl_id          | tpl_id          | bigint      | true     | scalar   | 模板ID                      |
| 23  | ak_source       | ak_source       | ARRAY       | true     | scalar   | -                           |
| 24  | tk_version      | tk_version      | bigint      | true     | scalar   | 版本号                      |
| 25  | tk_batch_no     | tk_batch_no     | bigint      | true     | scalar   | 批次号                      |
| 26  | fk_previous     | parent_id       | bigint      | true     | scalar   | 父订单ID（fk_previous）     |
| 27  | ck_branch       | ck_branch       | bigint      | true     | scalar   | 部门/分支                   |
| 28  | b_number        | sku_code        | text        | true     | scalar   | SKU编码                     |
| 29  | fk_editor       | fk_editor       | bigint      | true     | scalar   | 编辑人                      |
| 30  | type            | channel         | text        | true     | scalar   | 通道类型（DIRECT/TRANSFER） |

**原型语义补充**：

- `InboundItem.sku` → b_number（SKU编码）
- `InboundItem.name` → 无列，属SKU主数据外键引用
- `InboundItem.quantity` → 无列，属外接来源单据
- `InboundItem.receivedQuantity` → 无列，属收货记录汇总
- `InboundItem.unit` → 无列，属SKU主数据外键引用

---

## 3. DTO 4 视图设计

### 3.1 InboundOrder

**list**：id, order_no, supplier_id, supplier_name, warehouse_code, channel, qk_progress, lk_health, tpl_id, ak_source, created_at, dk_scene
**detail**：全部 32 列
**create**：code, notice, comments, ak_source, tpl_id, sort, qk_date_segm, qk_time_segm, qk_progress, progress_pct, schedule_pct, lk_health, dk_scene, dk_factor, dk_function
**update**：create + id

### 3.2 InboundBom

**list**：id, parent_id, code, b_number, tk_version, tk_batch_no, fk_editor, ck_branch, type, created_at
**detail**：全部 30 列
**create**：code, b_number, tk_version, tk_batch_no, fk_editor, ck_branch, type, tpl_id, ak_source, dk_scene, dk_factor, dk_function
**update**：create + id

---

## 4. 幻影风险评估

| 实体         | DB 列数 | 映射列数 | 幻影风险 |
| ------------ | ------- | -------- | -------- |
| InboundOrder | 32      | 32       | 无       |
| InboundBom   | 30      | 30       | 无       |

- 所有 DB 列均已映射，无幻影列
- 两表 relations 为空（无 DB 声明外键），has_many 关系通过 `fk_previous`（parent_id）建立
- `supplierId`/`supplierName`/`warehouseCode`/`expectedArrivalDate` 属主数据，不在本 Service 范围
