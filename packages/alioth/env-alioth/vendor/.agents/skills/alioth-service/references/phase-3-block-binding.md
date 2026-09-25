## Phase 3: Block 前端实现中绑定 Service API

### 3-1: Service API 路由命名规则

```
/api/service/{service-id}/{entity}
```

| Service ID | 路由前缀                   |
| ---------- | -------------------------- |
| catalog    | `/api/service/catalog/`    |
| valuation  | `/api/service/valuation/`  |
| commitment | `/api/service/commitment/` |

### 3-2: Block 共享 hooks 模式

Block 的 `shared/api/hooks.ts` 封装对 Service API 的调用，**使用 DTO 类型而非 DB entity 类型**：

```typescript
// Pre-Proc/{ns}/Sources/Apps/Blocks/unit-catalog/shared/api/hooks.ts
import { useQuery, useMutation } from '@tanstack/react-query';

const BASE = '/api/service/catalog';

export interface ProductSummary {
  id: string;
  name: string;
  status: ProductStatus;
  unitPrice?: Money;
}

export interface ProductDetail {
  id: string;
  name: string;
  pricing: PricingInfo;
  inventory: InventoryInfo;
  lifecycle: LifecycleInfo;
}

// list
export function useProductList(query: { page?: number; q?: string }) {
  return useQuery({
    queryKey: ['service-catalog', 'products', query],
    queryFn: async () => {
      const params = new URLSearchParams();
      if (query.page) params.set('page', String(query.page));
      if (query.q) params.set('q', query.q);
      const res = await fetch(`${BASE}/products?${params}`);
      return res.json() as Promise<PaginatedResponse<ProductSummary>>;
    },
  });
}

// detail
export function useProduct(id: string) {
  return useQuery({
    queryKey: ['service-catalog', 'products', id],
    queryFn: async () => {
      const res = await fetch(`${BASE}/products/${id}`);
      return res.json() as Promise<ApiResponse<ProductDetail>>;
    },
    enabled: !!id,
  });
}

// create
export function useCreateProduct() {
  return useMutation({
    mutationFn: async (data: CreateProductForm) => {
      const res = await fetch(`${BASE}/products`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(data),
      });
      return res.json() as Promise<ApiResponse<ProductDetail>>;
    },
  });
}
```

### 3-3: Block 页面绑定

Block 页面通过 hooks 消费 Service API，**字段名与 DTO 完全对齐**：

```tsx
// Pre-Proc/{ns}/Sources/Apps/Blocks/unit-catalog/flows/unit-management/index.tsx
function ProductListPage() {
  const { data, isLoading } = useProductList({ page: 1 });

  if (isLoading) return <Skeleton />;

  return (
    <table>
      {data?.items?.map((product) => (
        <tr key={product.id}>
          <td>{product.name}</td>
          <td>
            {product.unitPrice?.amount} {product.unitPrice?.currency}
          </td>
          <td>
            <StatusBadge status={product.status} />
          </td>
        </tr>
      ))}
    </table>
  );
}
```

### 3-4: 禁止

- ❌ Block 前端直接调用 `/api/service/{id}/` 而不通过 `shared/api/hooks.ts` 封装
- ❌ Block 前端使用 DB 列名（`notice`, `qk_price`, `sk_currency`）

### 3-6: Gateway 加载门禁（缺口 13）

Service API 开发完成后，**必须重启 Gateway 以确保新 API 被识别**：

```bash
# 重启 Gateway 使新 Service API 生效（MUST 显式传 --namespace：未定向时脚本按默认 ns 告警并回落）
bash scripts/gateway/restart-gateway.sh --namespace <ns>
```

**验收标准**：`curl -f http://localhost:8088/api/service/{service-id}/health || curl -f http://localhost:8088/api/service/{service-id}/` 返回 200。

### 3-5: DTO↔TS 字段一致门禁（缺口 8）

Phase 3 在 `tsc --noEmit` 通过后，**必须**依次执行：

```bash
# 1) 类型层：hooks.ts / 页面使用 DTO 类型（不得使用 DB entity 类型或裸列名）
npx tsc --noEmit

# 2) 原型↔生产 tsx 字段覆盖率（alioth-gui Track 4 同款；逐个受影响 Block 运行）
bun scripts/check/check-prototype-drift.ts <prototype.html> <production.tsx>
```

**门禁规则**：

| 检查                        | 退出码            | 处理                     |
| --------------------------- | ----------------- | ------------------------ |
| `check-prototype-drift.ts`  | 0 无漂移 / 1 漂移 | 修复字段名差异后重试     |
| `npx tsc --noEmit`          | 0 / 非 0          | 修类型错误后重试         |

> 仓库**无**「DTO↔TS 字段名对称」专用脚本（`--ns/--factor` 形式的对称门禁不存在）。Rust DTO 重构字段名后 MUST 人工核对 `hooks.ts` 接口定义（二者 1:1 透传），并以 `check-prototype-drift.ts` 的原型↔tsx 覆盖率作为结构化证据。

---
