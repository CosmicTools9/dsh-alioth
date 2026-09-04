# Customer Service — Design Notes

## DB Schema

Table: `isahl.zc_id_cate-organization` (17 columns)

## Column → DTO Mapping

| #   | Column          | Type (DB)   | Category | DTO Field       |
| --- | --------------- | ----------- | -------- | --------------- |
| 1   | created_at      | timestamptz | auto     | created_at      |
| 2   | updated_at      | timestamptz | auto     | updated_at      |
| 3   | id              | bigint      | auto     | id              |
| 4   | created_by_id   | bigint      | auto     | created_by_id   |
| 5   | updated_by_id   | bigint      | auto     | updated_by_id   |
| 6   | notice          | text        | scalar   | notice          |
| 7   | t_color_        | text        | scalar   | t_color_        |
| 8   | deleted_at      | timestamptz | auto     | deleted_at      |
| 9   | deleted_by_id   | bigint      | auto     | deleted_by_id   |
| 10  | code            | text        | scalar   | code            |
| 11  | o_number        | text        | auto     | o_number        |
| 12  | comments        | text        | scalar   | comments        |
| 13  | ak_benefit_user | ARRAY       | scalar   | ak_benefit_user |
| 14  | ak_permit_user  | ARRAY       | scalar   | ak_permit_user  |
| 15  | ak_access_user  | ARRAY       | scalar   | ak_access_user  |
| 16  | enable          | boolean     | scalar   | enable          |
| 17  | c_sort_         | bigint      | scalar   | c_sort_         |

## DTO Design (4 structs)

### CustomerListItem

Lightweight: `id, code, o_number, c_sort_, enable, ak_benefit_user, ak_access_user, created_at`

### CustomerDetail

All 17 columns.

### CustomerCreate

All writeable scalars: `code, o_number, notice, t_color_, comments, ak_benefit_user, ak_permit_user, ak_access_user, enable, c_sort_` (10 fields, no id/auto fields).

### CustomerUpdate

`id + CustomerCreate fields` (11 fields).

## Notes

- ak_* fields are ARRAY (Vec<i64>) per DB schema.
- o_number: field_category=auto but no default → treat as scalar in DTO (nullable text).
- All qk_/ck_/fk_/lk_/dk_/tk_ fields → Option<i64>.
- created_at/updated_at non-null → DateTime<Utc>.
- deleted_at nullable → Option<DateTime<Utc>>.
