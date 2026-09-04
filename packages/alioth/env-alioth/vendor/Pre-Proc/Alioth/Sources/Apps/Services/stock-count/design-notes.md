# stock-count Service Design Notes

## 4-Table Column Mapping

### zc_id_even-counting (29 cols)

| #   | column          | data_type   | nullable | category | DTO mapping                      |
| --- | --------------- | ----------- | -------- | -------- | -------------------------------- |
| 1   | created_at      | timestamptz | NOT NULL | auto     | Detail, Update                   |
| 2   | updated_at      | timestamptz | NOT NULL | auto     | Detail, Update                   |
| 3   | id              | bigint      | NOT NULL | auto     | ListItem, Detail, Update         |
| 4   | created_by_id   | bigint      | YES      | auto     | Detail, Update                   |
| 5   | updated_by_id   | bigint      | YES      | auto     | Detail, Update                   |
| 6   | notice          | text        | YES      | scalar   | Detail, Update                   |
| 7   | t_color_        | text        | YES      | scalar   | Detail, Update                   |
| 8   | deleted_at      | timestamptz | YES      | auto     | —                                |
| 9   | deleted_by_id   | bigint      | YES      | auto     | —                                |
| 10  | code            | text        | YES      | scalar   | ListItem, Detail, Create, Update |
| 11  | o_number        | text        | YES      | scalar   | ListItem, Detail, Create, Update |
| 12  | comments        | text        | YES      | scalar   | Detail, Update                   |
| 13  | ak_benefit_user | ARRAY       | YES      | scalar   | Detail, Create, Update           |
| 14  | ak_permit_user  | ARRAY       | YES      | scalar   | Detail, Create, Update           |
| 15  | ak_access_user  | ARRAY       | YES      | scalar   | Detail, Create, Update           |
| 16  | projection      | text        | YES      | scalar   | Detail, Update                   |
| 17  | _f_             | text        | YES      | scalar   | Detail, Update                   |
| 18  | _t_             | text        | YES      | scalar   | Detail, Update                   |
| 19  | dk_scene        | bigint      | YES      | scalar   | ListItem, Detail, Create, Update |
| 20  | dk_factor       | bigint      | YES      | scalar   | Detail, Create, Update           |
| 21  | dk_function     | bigint      | YES      | scalar   | Detail, Create, Update           |
| 22  | tpl_id          | bigint      | YES      | scalar   | Detail, Create, Update           |
| 23  | ak_source       | ARRAY       | YES      | scalar   | Detail, Create, Update           |
| 24  | qk_date         | bigint      | YES      | scalar   | ListItem, Detail, Create, Update |
| 25  | fk_place        | bigint      | YES      | scalar   | Detail, Create, Update           |
| 26  | fk_subject      | bigint      | YES      | scalar   | Detail, Create, Update           |
| 27  | timeline        | jsonb       | YES      | scalar   | Detail, Update                   |
| 28  | fk_storage      | bigint      | YES      | scalar   | Detail, Create, Update           |
| 29  | summary         | text        | YES      | scalar   | Detail, Update                   |

### zc_id_deta-counting (33 cols)

| #   | column          | data_type   | nullable | category | DTO mapping                      |
| --- | --------------- | ----------- | -------- | -------- | -------------------------------- |
| 1   | created_at      | timestamptz | NOT NULL | auto     | Detail, Update                   |
| 2   | updated_at      | timestamptz | NOT NULL | auto     | Detail, Update                   |
| 3   | id              | bigint      | NOT NULL | auto     | ListItem, Detail, Update         |
| 4   | created_by_id   | bigint      | YES      | auto     | Detail, Update                   |
| 5   | updated_by_id   | bigint      | YES      | auto     | Detail, Update                   |
| 6   | notice          | text        | YES      | scalar   | Detail, Update                   |
| 7   | t_color_        | text        | YES      | scalar   | Detail, Update                   |
| 8   | deleted_at      | timestamptz | YES      | auto     | —                                |
| 9   | deleted_by_id   | bigint      | YES      | auto     | —                                |
| 10  | code            | text        | YES      | scalar   | ListItem, Detail, Create, Update |
| 11  | o_number        | text        | YES      | scalar   | ListItem, Detail, Create, Update |
| 12  | comments        | text        | YES      | scalar   | Detail, Update                   |
| 13  | ak_benefit_user | ARRAY       | YES      | scalar   | Detail, Create, Update           |
| 14  | ak_permit_user  | ARRAY       | YES      | scalar   | Detail, Create, Update           |
| 15  | ak_access_user  | ARRAY       | YES      | scalar   | Detail, Create, Update           |
| 16  | projection      | text        | YES      | scalar   | Detail, Update                   |
| 17  | _f_             | text        | YES      | scalar   | Detail, Update                   |
| 18  | _t_             | text        | YES      | scalar   | Detail, Update                   |
| 19  | dk_scene        | bigint      | YES      | scalar   | Detail, Create, Update           |
| 20  | dk_factor       | bigint      | YES      | scalar   | Detail, Create, Update           |
| 21  | dk_function     | bigint      | YES      | scalar   | Detail, Create, Update           |
| 22  | tpl_id          | bigint      | YES      | scalar   | Detail, Create, Update           |
| 23  | ak_source       | ARRAY       | YES      | scalar   | Detail, Create, Update           |
| 24  | qk_date         | bigint      | YES      | scalar   | ListItem, Detail, Create, Update |
| 25  | ck_category     | bigint      | YES      | scalar   | Detail, Create, Update           |
| 26  | fk_list         | bigint      | YES      | scalar   | Detail, Create, Update           |
| 27  | fk_biller       | bigint      | YES      | scalar   | Detail, Create, Update           |
| 28  | fk_production   | bigint      | YES      | scalar   | Detail, Create, Update           |
| 29  | fk_storage      | bigint      | YES      | scalar   | Detail, Create, Update           |
| 30  | qk_qty          | bigint      | YES      | scalar   | Detail, Create, Update           |
| 31  | qk_w_qty        | bigint      | YES      | scalar   | Detail, Create, Update           |
| 32  | qk_v_qty        | bigint      | YES      | scalar   | Detail, Create, Update           |
| 33  | fk_voucher      | bigint      | YES      | scalar   | Detail, Update                   |

### zc_id_stus-counting (17 cols)

| #   | column          | data_type    | nullable | category | DTO mapping                      |
| --- | --------------- | ------------ | -------- | -------- | -------------------------------- |
| 1   | created_at      | timestamptz  | NOT NULL | auto     | Detail, Update                   |
| 2   | updated_at      | timestamptz  | NOT NULL | auto     | Detail, Update                   |
| 3   | id              | bigint       | NOT NULL | auto     | ListItem, Detail, Update         |
| 4   | created_by_id   | bigint       | YES      | auto     | Detail, Update                   |
| 5   | updated_by_id   | bigint       | YES      | auto     | Detail, Update                   |
| 6   | notice          | text         | YES      | scalar   | Detail, Update                   |
| 7   | t_color_        | text         | YES      | scalar   | Detail, Update                   |
| 8   | deleted_at      | timestamptz  | YES      | auto     | —                                |
| 9   | deleted_by_id   | bigint       | YES      | auto     | —                                |
| 10  | code            | text         | YES      | scalar   | ListItem, Detail, Create, Update |
| 11  | o_number        | text         | YES      | scalar   | ListItem, Detail, Create, Update |
| 12  | comments        | text         | YES      | scalar   | Detail, Update                   |
| 13  | ak_benefit_user | ARRAY        | YES      | scalar   | Detail, Create, Update           |
| 14  | ak_permit_user  | ARRAY        | YES      | scalar   | Detail, Create, Update           |
| 15  | ak_access_user  | ARRAY        | YES      | scalar   | Detail, Create, Update           |
| 16  | enable          | boolean      | YES      | scalar   | Detail, Update                   |
| 17  | flag            | USER-DEFINED | YES      | scalar   | Detail, Update                   |

### zc_id_counting_r_cnt-status (14 cols) — relationship table, no CRUD DTOs

| #   | column        | data_type   | nullable | category |
| --- | ------------- | ----------- | -------- | -------- |
| 1   | created_at    | timestamptz | NOT NULL | auto     |
| 2   | updated_at    | timestamptz | NOT NULL | auto     |
| 3   | id            | bigint      | NOT NULL | auto     |
| 4   | created_by_id | bigint      | YES      | auto     |
| 5   | updated_by_id | bigint      | YES      | auto     |
| 6   | notice        | text        | YES      | scalar   |
| 7   | t_color_      | text        | YES      | scalar   |
| 8   | deleted_at    | timestamptz | YES      | auto     |
| 9   | deleted_by_id | bigint      | YES      | auto     |
| 10  | code          | text        | YES      | scalar   |
| 11  | ref_left      | bigint      | YES      | scalar   |
| 12  | ref_right     | bigint      | YES      | scalar   |
| 13  | comments      | text        | YES      | scalar   |
| 14  | status_date   | timestamptz | YES      | scalar   |

## Entity ↔ Table Mapping

- `StockCount` → `isahl.zc_id_even-counting`
- `StockCountDetail` → `isahl.zc_id_deta-counting`
- `StockCountStatus` → `isahl.zc_id_stus-counting`
- `CountingRCountStatus` (relationship) → `isahl.zc_id_counting_r_cnt-status` (declared as relationship in ontology)

## Phantom Column Check

All 4 tables verified against DB — zero phantom columns. Counts match raw-sql.
