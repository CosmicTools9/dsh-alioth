## Track 3: 现有 Service 优化

### 内聚性审计

检查 Service 单元是否混杂了不属于其域的业务：

| 信号                | 检查方法                             | 操作         |
| ------------------- | ------------------------------------ | ------------ |
| handler > 5 个      | 统计 handler 文件数                  | 评估拆分     |
| 操作 > 3 张不同表   | `grep "FROM zc_id_" repository/*.rs` | 检查是否同域 |
| DTO 依赖 > 3 个     | 统计 `factor-*-dto` 依赖             | 评估粒度     |
| Service 编码 > 3 个 | 查 `service.json` 的 `factors` 字段  | 评估拆分     |

---
