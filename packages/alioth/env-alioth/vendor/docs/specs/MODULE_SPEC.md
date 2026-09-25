---
title: 前端开发规约
alias: MODULE_SPEC
status: implemented
layer: executable
last_verified: 2026-06-12
depends:
  - DTO_DESIGN_SPEC.md (前端字段对齐 DTO)
  - MODULE_FRONTEND_SPEC.md (前端组件规范)
---

# 前端开发规约

> **版本**: v1.6.0 | **最后更新**: 2026-06-12
>
> **适用范围**: AliothStudio 所有前端应用（Meta / Gateway / SSO / Modules）
> **技术栈**: TypeScript / React / React Router / Jotai v2

---

## 1. 认证与路由守卫

所有涉及认证判断的路由守卫、登录页、iframe 预览入口，**必须等待认证状态加载完成（`isLoading === false`）后再执行跳转或重定向**。

### 正确模式

```tsx
// 路由守卫 — 先等 isLoading，再判断 isAuthenticated
function AuthGuard(): React.ReactElement | null {
  const { isAuthenticated, isLoading } = useAuth();
  const location = useLocation();

  if (isLoading) {
    return <LoadingSpinner />;
  }

  if (!isAuthenticated) {
    return <Navigate to="/login" state={{ from: location }} replace />;
  }

  return <Outlet />;
}

// 登录页 — 等加载完成后再判断是否需要重定向
useEffect(() => {
  if (!isLoading && isAuthenticated) {
    navigate(from || '/', { replace: true });
  }
}, [isLoading, isAuthenticated, navigate, from]);

// iframe 预览入口 — 同样等待 isLoading
function PreviewApp(): React.ReactElement | null {
  const { isAuthenticated, isLoading } = useAuth();
  if (isLoading) {
    return <LoadingSpinner />;
  }
  if (!isAuthenticated) {
    window.location.replace('/login');
    return <div>未登录，正在跳转...</div>;
  }
}
```

### 规范

- 路由守卫必须先检查 `isLoading`，再判断 `isAuthenticated`，避免初始 `false` 导致误跳转
- 登录页 `useEffect` 必须包含 `isLoading` 依赖，不得在认证状态加载前执行 `navigate('/')`
- iframe 预览入口必须在认证状态恢复完成后才执行跳转，不得在恢复前 `replace('/login')`

---

## 2. 大整数处理

AliothStudio 使用 `zuid`（`BIGINT`）作为主键，典型值远超 JavaScript `Number` 安全整数上限（`2^53 - 1 ≈ 9e15`）。**前端 JS `Number` 无法精确表示 `zuid`，会导致行标记错乱、更新请求命中错误记录等隐蔽 bug。**

### 防御体系（三层）

#### 1. 后端义务（首选）

通用表数据 API 已通过 `row_to_json` 将 `id`、`zuid` 列序列化为 JSON String。其他接口若返回大整数主键同样应使用字符串。

#### 2. 框架级兜底（API Client）

`@alioth/api` 的 `ApiClient` 已配置 **BigInt-safe JSON 解析**：当后端返回的 JSON 数值超过 `Number.MAX_SAFE_INTEGER` 时，自动转为字符串。此机制作为后端未完全覆盖时的兜底防线。

```ts
// Framework/frontend/api/src/client.ts
JSON.parse(responseText, (_key, value) => {
  if (typeof value === 'number' && !Number.isSafeInteger(value)) {
    return String(value); // 1134907106097374215 -> "1134907106097374215"
  }
  return value;
});
```

> **注意**：此兜底仅覆盖数值类型。若后端已将字段序列化为字符串，则保持字符串不变。

#### 3. 前端义务（必须遵守）

- **所有涉及 `id`、`zuid`、主键、外键的变量/属性，类型声明必须为 `string | number`**（门禁：`scripts/check/check-id-json-precision.ts`）
- 行内编辑的 `editingRowId`、选中项 `selectedId`、路由参数中的 ID，全部按字符串处理
- 比较 ID 相等时，统一使用 `String(a) === String(b)`（门禁：`scripts/check/check-id-json-precision.ts`）
- API 请求中的路径参数直接传入字符串，禁止先转 `Number` 再传

### 正确模式

```tsx
// 类型声明兼容字符串
const [editingRowId, setEditingRowId] = React.useState<string | number | null>(null);

// ID 比较时统一转字符串
const isSameRow = String(row.id) === String(editingRowId);

// API 请求直接传字符串 id
await tableDataApi.updateRecord(tableName, String(id), payload);
```

### 规范

- ID 必须保持字符串类型，不得使用 `Number(id)` 转换
- ID 比较必须使用 `String(a) === String(b)`
- URL 拼接中 ID 必须保持字符串，不得 `Number` 转换后拼接
- 所有大整数 ID 使用框架 `ApiClient` 处理，不得依赖 `response.json()` 默认行为（未使用 `ApiClient` 的自定义模块除外）

---

## 3. 前端 `_refs` 字段与关联显示

### 数据来源

后端 `list_refs` / `get_refs` 返回的记录中嵌入 `_refs` JSONB 字段。

### 前端取值规则

```tsx
// 从 _refs 取显示值
const displayValue = record._refs?.currency?.notice ?? '-';

// 遍历 _refs 渲染关联数据
{
  Object.entries(record._refs ?? {}).map(([key, ref]) => <td key={key}>{ref.notice}</td>);
}
```

**所有涉及 FK 关联的显示值必须从 `_refs.*.notice` 取用，禁止前端硬编码映射表。**

> _\*qk_* 与 FK 关联的 \_refs 显示路径区别_*：
>
> - **FK 关联**（如 `fk_currency`）：显示值统一从 `_refs.{field}.notice` 取用
> - **刻度字段 qk_***（如 `qk_price`、`qk_date`）：显示值从 `_refs.{field}.{sub_field}` 取用（如 `_refs.qk_price.mark`、`_refs.qk_date.date`），详见 `DTO_DESIGN_SPEC.md` §三.1

---

## 4. 前端数据建模与字段约束

### 4.0 三层命名空间（术语统一）

> **来源**：本节由 `spec-audit` 报告冲突 1.1 引入（2026-06-10）。术语统一，**禁止混用**。

前端 ↔ 后端 ↔ 数据库存在三层独立的命名空间，**每层有自己的命名规则和职责**：

| 层级   | 命名空间                 | 形态               | 维护方                                  | 例子                                             |
| ------ | ------------------------ | ------------------ | --------------------------------------- | ------------------------------------------------ |
| **L1** | 物理列 (Physical Column) | snake_case + 前缀  | DDL（不可变）                           | `notice`、`fk_country`、`qk_price`、`s_notice`   |
| **L2** | DTO 字段 (DTO Field)     | camelCase 业务语义 | 后端 `dto.rs` (Create*/Update* Request) | `name`、`country`、`price`（= ScalarPriceValue） |
| **L3** | 业务模型 (Domain Model)  | camelCase 业务概念 | 前端 `types/*.ts`                       | `name`、`category`、`country`                    |

**映射规则**（单向、不重不漏）：

| L1 物理列     | L2 DTO 字段 | 类型                       | 说明                                             |
| ------------- | ----------- | -------------------------- | ------------------------------------------------ |
| `notice`      | `name`      | `String`                   | 主体名称                                         |
| `code`        | `code`      | `String`                   | 内部编码（同名）                                 |
| `comments`    | `comments`  | `Option<String>`           | 备注                                             |
| `fk_country`  | `country`   | `Option<i64>`              | 去掉 `fk_` 前缀                                  |
| `ck_category` | `category`  | `Option<i64>`              | 去掉 `ck_` 前缀                                  |
| `sk_unit`     | `unit`      | `Option<i64>`              | 去掉 `sk_` 前缀                                  |
| `qk_price`    | `price`     | `Option<ScalarPriceValue>` | **结构化值对象**（详见 `DTO_DESIGN_SPEC §2.2`） |
| `qk_date`     | `date`      | `Option<ScalarDateValue>`  | 结构化值对象                                     |
| `s_notice`    | —           | —                          | **禁止**进入 DTO（标量子表内部字段）             |

**L2 → L3**：1:1 透传，API 层不转换。

**术语禁用清单**（避免与本节冲突）：

- 描述 L1 时使用"物理列"（而非"前端字段名"）
- 描述 L3 时使用"业务模型 / types"（而非"DTO 字段"）
- 描述 L2 时统一为"DTO 字段"——由后端定义、前端提交

### 4.1 建模原则

前端 `types/*.ts`（L3 业务模型）**基于一般性的业务语义**进行本体的 JSON 模型设计，DB 模型字段和关系作为**参考而非强制约束**。

- 大部分时候 L3 业务模型与 L1 物理表模型在宏观本体上一致
- 但由于关系存储时数据结构设计的巨大差异，**不可直接进行逐列绑定**
- 原因：文档对象模型（JSON）和关系对象模型（Table）的存储结构本质不同
- L3 业务模型字段名 **=** L2 DTO 字段名（API 层 1:1 透传），**≠** L1 物理列名

```typescript
// ✅ 正确：L3 业务模型（types/*.ts）= L2 DTO 字段名
interface OrganizationFormData {
  name: string; // L2: name  ←  L1: notice
  code: string; // L2: code  ←  L1: code (同名)
  category: string; // L2: category  ←  L1: ck_category
  country: string; // L2: country  ←  L1: fk_country
}

interface OrganizationFormData {
  notice: string; // L1 物理列，前端业务模型不应使用
  ck_category: string; // L1 物理列名
  fk_country: number; // L1 物理列名（且 number 类型违反 §2 大整数处理）
}
```

### 4.2 可编辑字段约束

Alioht 的 lifecycle 子表（继承自 `zc_id_lifecycle` 的表）中，**本体数据有且仅有以下三个 text 字段允许输入和编辑**：

> **术语修正**（2026-06-10）：表格第二列"前端字段名"实指 **L2 DTO 字段名**（即后端 Create*/Update* Request 字段名 = 前端 zod schema 字段名）。前端 `types/*.ts`（L3 业务模型）直接复用 L2 字段名，**不另起名**。详见 §4.0 三层命名空间。

| L1 物理列  | L2 DTO 字段名 | 含义 | 说明                                                                     |
| ---------- | ------------- | ---- | ------------------------------------------------------------------------ |
| `notice`   | `name`        | 名称 | 主体名称，自由文本                                                       |
| `code`     | `code`        | 编码 | 内部编码，自由文本（**是否填入决策见 `AGENTS.md` "code 字段分层规则"**） |
| `comments` | `comments`    | 备注 | 补充说明，自由文本                                                       |

**所有其他 text 字段**（含 `o_number`、`domain_`、`x_code`、`x_prefix` 等）**均不接受外部输入数据**。

这些字段属于 Alioth 维度派生体系，由关系建模的维度实体（Dimension Entity）通过关联派生：

```text
┌─────────────────────┐
│    组织 (SubjOrg)    │ ← lifecycle 子表
│                     │
│  notice  ← 用户输入  │
│  code    ← 用户输入  │
│  comments ← 用户输入  │
│                     │
│  domain_ ← 维度派生  │ ← 不可直接输入
│  x_code  ← 维度派生  │ ← 不可直接输入
│  ...      ← 维度派生  │ ← 不可直接输入
└─────────────────────┘
```

### 4.3 关联字段输入规约

> 字段前缀语义、DDL 类型与请求格式（结构化值对象 / ID）的**唯一正本见 `DTO_DESIGN_SPEC.md` §1.1 / §2.8 / §3**；本节只定义前端交互方式，不复制类型与格式细节。

| 字段前缀 | 交互方式                     | 说明                                                       |
| -------- | ---------------------------- | ---------------------------------------------------------- |
| `ck_*`   | 下拉单选（SearchableSelect） | 维度分类选择器                                             |
| `sk_*`   | 下拉单选（SearchableSelect） | 单位选择器（zc_id_unit）                                   |
| `tk_*`   | 下拉单选 + 支持快速新增      | 类型键选择器（可一键创建新类型）                           |
| `fk_*`   | 业务搜索选择器               | 外键引用，通过关联实体列表 API 搜索                        |
| `qk_*`   | 拆为 2 字段编辑              | `mark` 数值输入 + `sk_unit` 下拉单选（默认继承主对象单位） |

> _\*qk_* 前后端交互格式__：前端将 qk__ 拆为 `mark` 数值输入 + `sk_unit` 下拉单选提交。后端 DTO 通过结构化值对象 `{ value: ... }` 接收，映射层由 `REFERENCE_RESOLVER` 自动处理。详见 `DTO_DESIGN_SPEC.md` §二.1。

### 4.4 新建/编辑交互分级

根据实体可编辑字段总数（不含系统写入/应用层绑定字段），选择交互方式：

| 字段数量 | 交互方式          | 说明                   |
| -------- | ----------------- | ---------------------- |
| < 8      | 表格空白行内编辑  | 直接在表格行中编辑新增 |
| 8-20     | 抽屉表单（Sheet） | 右侧滑出抽屉           |
| > 20     | 单页表单          | 独立路由完整表单页     |

表单字段必须遵循**数据内在关系和必要性排序**，而非物理列顺序。

## 5. 状态管理

| 状态类型   | 技术                    | 用途                                 |
| ---------- | ----------------------- | ------------------------------------ |
| 服务端状态 | `@tanstack/react-query` | API 数据缓存、后台同步               |
| 客户端状态 | Jotai v2                | 全局 UI 状态、用户偏好、表单本地状态 |

状态管理唯一选型 = 上表两项（Zustand/Redux/Recoil 等由门禁拦截；门禁：`scripts/check/check-state-management-libs.ts`）。

---

## 6. Gateway 布局架构

Gateway 采用 **双重视角** 的布局系统，详见 [`GATEWAY_DESIGN_SPEC.md`](GATEWAY_DESIGN_SPEC.md) §3。

### 6.1 核心布局组件

| 组件                | 文件                                                             | 职责                                                                                        |
| ------------------- | ---------------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| `TopBar`（Gateway） | `Gateway/frontend/src/components/TopBar.tsx`                     | 组合 FrameworkTopBar，按视角切换内容                                                        |
| `FrameworkTopBar`   | `Framework/frontend/components/src/components/layout/TopBar.tsx` | 顶栏外壳，提供 `logo` / `tabs` / `breadcrumbs` / `searchSlot` / `actions` / `userMenu` 插槽 |
| `Navigation`        | `Gateway/frontend/src/components/Navigation.tsx`                 | Gateway 视角左侧模块导航栏                                                                  |
| `ModuleTabs`        | `Gateway/frontend/src/components/ModuleTabs.tsx`                 | 应用视角下顶栏左侧模块标签页                                                                |
| `MainLayout`        | `Gateway/frontend/src/layouts/MainLayout.tsx`                    | 主布局，组合 TopBar + Navigation + Content + WorkspaceDock                                  |

### 6.2 视角检测

`useAppContext` hook（`Gateway/frontend/src/hooks/useAppContext.ts`）负责：

- 从 URL `pathname` 匹配 `/api/apps` 返回的应用 `config.modules` 列表
- 返回 `{ currentApp, appModules, isAppPerspective, isLoading }`
- 最长前缀优先匹配：`/employee` 匹配模块 `employee` → 所属应用成为 `currentApp`
- 首页 `/` 或不匹配任何应用模块时，`isAppPerspective === false`

### 6.3 AI Workspace 上下文传递

EmpAgent 面板（`AIWorkspace` / `AIChatPanel`）通过 `PageContextModule` 自动获取当前页面上下文：

```tsx
// 页面组件注册上下文
function ProductListPage() {
  useProvideAIContext({
    module: "inventory",
    page: "产品列表",
    currentData: { filter: "品类=电子", resultCount: 156 },
    availableOperations: ["新建产品", "批量导入", "导出报表"],
  });
  return <DataTable ... />;
}
```

**约束**：

- `PageContextModule` 是模块级单例，所有模块共享同一实例
- 路由切换时自动清除旧上下文（`useProvideAIContext` 的 cleanup）
- 字段自动裁剪：`extraContext` 超过 1000 字符截断
- 支持 Agent 特定模板渲染（`general` / `form_filling` / `data_analysis`）

### 6.4 关键规则

- **应用视角下 Navigation 侧栏只显示当前 App 的模块导航（不显示应用列表）**：模块导航通过 TopBar 左侧 `ModuleTabs` 呈现；Navigation 侧栏只保留当前 App 的 `useModuleSidebar()` 推送的 navItems。
- **应用视角下品牌标识只出现一次**：TopBar 已在 `logo` 槽展示应用名（`currentApp.name`），模块 Sidebar 品牌 block 已整体移除（详见 §11.11.6），禁止在两个位置同时出现品牌元素。
- **Logo 始终可点击返回 Gateway 视角**：`href="/"`
- **FrameworkTopBar 的 `tabs` 插槽仅在应用视角下传入**，Gateway 视角下为 `undefined`

> **ESM 集成契约参考**: Block → Module → App 三层的 ESM 职责边界（各层渲染什么、滚动容器归属、CSS 变量级联）定义在 `specs/block-module-app-integration/spec.md`（`openspec/changes/spec-block-module-app-shell-contract/specs/block-module-app-integration/spec.md`）。原型的 Module embedded 模式与生产应用视角是两条独立渲染路径，详见该 spec 的 `prototype-embedded-vs-production` 小节。

## 7. 前端 API Hooks 规范

### 7.1 工厂模式

所有模块统一使用 `createPaginatedCrudHooks` 工厂生成 CRUD hooks：

```typescript
// Pre-Proc/{ns}/Sources/Apps/Modules/{name}/frontend/src/api/entity.ts
import { createPaginatedCrudHooks } from '@alioth/api';
import type { Entity, CreateEntityDTO, UpdateEntityDTO } from './types';

export const entityHooks = createPaginatedCrudHooks<Entity, CreateEntityDTO, UpdateEntityDTO>({
  basePath: '/api/module/entities',
  resourceName: 'entities',
  queryKey: ['module', 'entities'],
});
```

### 7.2 文件命名

| 规则   | 说明                                                                                    |
| ------ | --------------------------------------------------------------------------------------- |
| 文件名 | `api/{entity_name}.ts`（snake_case）                                                    |
| 导出   | 默认导出 hooks 对象：`export const { entityName }Hooks = createPaginatedCrudHooks(...)` |
| 类型   | DTO 类型定义在 `api/types.ts` 或同文件内联                                              |

### 7.3 hooks 对象结构

`createPaginatedCrudHooks` 返回：

```typescript
{
  useList: (query: ListQuery) => UseQueryResult<PaginatedResponse<Entity>>;
  useGet: (id: string | number) => UseQueryResult<Entity>;
  useCreate: () => UseMutationResult<Entity, Error, CreateEntityDTO>;
  useUpdate: (id: string | number) => UseMutationResult<Entity, Error, UpdateEntityDTO>;
  useDelete: () => UseMutationResult<void, Error, string | number>;
}
```

### 7.4 Mutation 约定

```typescript
// 统一 onSuccess 模式
const createMutation = entityHooks.useCreate();
const handleSubmit = (data: CreateEntityDTO) => {
  createMutation.mutate(data, {
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['module', 'entities'] });
      notify.success('创建成功');
      navigate('/module/entities');
    },
    onError: (error) => {
      notify.apiError(error);
    },
  });
};
```

**强制规则**：
+- mutation `onSuccess` 中**必须**调用 `queryClient.invalidateQueries` 刷新列表缓存
+- mutation `onError` 中**必须**使用 `notify.apiError(error)` 展示错误
+- 创建/更新成功后**必须**导航回列表页

### 7.5 queryKey 命名空间

| 层级   | 格式                           | 示例                               |
| ------ | ------------------------------ | ---------------------------------- |
| 模块级 | `["{module}"]`                 | `["inventory"]`                    |
| 实体级 | `["{module}", "{entity}"]`     | `["inventory", "products"]`        |
| 详情级 | `["{module}", "{entity}", id]` | `["inventory", "products", "123"]` |

**禁止**：
+- queryKey 中使用裸字符串数组（如 `["products"]`），必须带模块前缀以避免跨模块缓存碰撞
+- 手动拼接 queryKey 字符串（如 `` `${module}-${entity}` ``），必须使用分层数组

### 7.6 聚合映射层

当实体需要 JOIN 多个表时，API 层负责聚合，返回前端语义类型：

```typescript
async function fetchAggregatedEntities(): Promise<PaginatedData<Entity>> {
  const rawList = await fetchRawEntities();
  const entities = await Promise.all(
    rawList.map(async (raw) => {
      const items = await fetchRelatedItems(raw.id);
      return mapRawEntity(raw, items); // 映射为前端语义类型
    }),
  );
  return { list: entities, count: entities.length };
}
```

- 映射函数职责：后端物理列 → 前端语义字段（`notice` → `name`、`created_at` → `createdAt`）。
- 禁止在前端组件中直接拼接 SQL 或执行数据库查询；禁止硬编码 API URL（使用 `apiClient` 或环境变量）；禁止在 API hook 中操作 DOM 或路由。

---

## 8. 国际化（i18n）规范

### 8.1 文件结构

```
Pre-Proc/{ns}/Sources/Apps/Modules/{name}/frontend/src/locales/
├── zh-CN.json    # 简体中文
└── en.json       # 英文
```

所有模块均包含此双语结构。

### 8.2 键命名空间

```
{module}.{section}.{key}
```

| 段          | 含义             | 示例                                       |
| ----------- | ---------------- | ------------------------------------------ |
| `{module}`  | 模块名（目录名） | `inventory`、`contacts`、`orders`          |
| `{section}` | 页面/功能区      | `nav`、`form`、`list`、`action`、`common`  |
| `{key}`     | 具体文本键       | `title`、`save`、`delete`、`confirmDelete` |

> 禁止使用物理表名前缀（如 `zc_id_*`），统一使用模块目录名确保简洁可读。

### 8.3 键值约束

| 规则                  | 说明                                     | 违规示例                                                         |
| --------------------- | ---------------------------------------- | ---------------------------------------------------------------- |
| **段名必须为英文**    | key 路径中所有段名使用英文               | `access.dashboard.权限总览`                                      |
| **值使用当前语言**    | value 保留对应语言的自然语言文本         | `"access.dashboard.permissionOverview": "权限总览"`              |
| **硬编码字符串**  | JSX/TSX 中所有用户可见文本必须通过 `t()`（门禁：`scripts/check/check-i18n-coverage.ts`、`scripts/check/check-i18n-jsx-chinese.ts`） | `<span>返回列表</span>` → `<span>{t("xxx.backToList")}</span>`   |
| **禁止硬编码 locale** | 日期/数字格式化从 i18n 配置读取 locale   | `.toLocaleDateString('zh-CN')`                                   |
| **禁止嵌套 JSON**     | locales 文件使用 flat dot-separated 结构 | `{ "nav": { "title": "..." } }` → `"inventory.nav.title": "..."` |

### 8.4 标准 section 清单

| section        | 用途          | 必须包含                                      |
| -------------- | ------------- | --------------------------------------------- |
| `nav`          | 导航标题      | `title`（模块名）、每个实体的 `list`/`create` |
| `list`         | 列表页        | 列标题、搜索占位符、空状态文本                |
| `form`         | 表单页        | 字段标签、验证消息、提交按钮文本              |
| `crud.actions` | CRUD 操作按钮 | `create`、`edit`、`delete`、`save`、`cancel`  |
| `common`       | 通用文本      | `loading`、`error`、`noData`、`confirmDelete` |

> `crud.actions` 使用裸 `{section}.{key}` 形式（如 `crud.actions.create`），由 Framework `@alioth/components` 提供默认值，模块可覆盖。

### 8.5 示例

```json
{
  "inventory.nav.title": "仓库管理",
  "inventory.nav.products": "产品库存",
  "inventory.list.searchPlaceholder": "搜索仓库...",
  "inventory.list.emptyState": "暂无仓库记录",
  "inventory.form.warehouseName": "仓库名称",
  "inventory.form.capacity": "容量",
  "inventory.form.save": "保存",
  "inventory.crud.actions.create": "新增仓库",
  "inventory.crud.actions.delete": "删除",
  "inventory.crud.actions.confirmDelete": "确认删除该仓库？",
  "common.loading": "加载中...",
  "common.noData": "暂无数据"
}
```

### 8.6 组件字典合并

所有模块**必须**合并 `@alioth/components` 的共享字典：

````typescript
// App.tsx
import { componentsZhCN, componentsEn } from "@alioth/components";
import zhCN from "./locales/zh-CN.json";
import en from "./locales/en.json";

const mergedZhCN = { ...componentsZhCN, ...zhCN };
const mergedEn = { ...componentsEn, ...en };

export function App(): React.ReactElement {
  return (
    <ModuleI18nShell zhCN={mergedZhCN} en={mergedEn}>
      <Routes>...</Routes>
    </ModuleI18nShell>
  );
}

模块自身的键优先级高于组件字典（后展开覆盖先展开）。

### 8.7 跨包组件库的 locale 暴露

面向其他应用/模块**导出可复用组件**的共享包（典型：`@alioth/sso-frontend`，未来可能扩展的认证/审计前端包）必须暴露自有 locales，便于宿主 app 显式合并。

#### 8.7.1 `package.json` `exports` 声明

```json
{
  "name": "@alioth/<name>-frontend",
  "exports": {
    ".": "./src/pages/index.ts",
    "./components": "./src/components/index.ts",
    "./hooks": "./src/hooks/index.ts",
    "./locales/zh-CN": "./src/locales/zh-CN.json",
    "./locales/en": "./src/locales/en.json"
  }
}
#### 8.7.2 键命名空间

共享包使用包名作为 key 第一段（**非**模块名）：

| 包 | 命名空间 | 示例 |
|----|---------|------|
| `@alioth/sso-frontend` | `sso.*` | `sso.login.divider`、`sso.error.loginFailed` |
| `@alioth/<pkg>-frontend` | `<pkg>.*` | `<pkg>.<section>.<key>` |

避免与宿主 app（`login.*`、`nav.*`）的键名冲突。

#### 8.7.3 文件结构与 §8.1-8.4 一致

````

SSO/frontend/src/locales/
├── zh-CN.json # 简体中文
└── en.json # 英文

````

flat dot-separated 结构，禁止嵌套。

### 8.8 宿主 app 加载共享包 locale

宿主 app（`Gateway`、`Meta`，或业务模块的 `App.tsx`）使用其他包的可复用组件时，**必须**显式 import 该包的 locales 并合并进 `I18nProvider.initialDictionaries`。**禁止**假设共享包会自动注入（`I18nCore` 的字典是宿主级单例，跨包无法感知）。

```typescript
// Gateway/frontend/src/main.tsx
import { I18nProvider } from "@alioth/i18n";
import { componentsZhCN, componentsEn } from "@alioth/components";
import gatewayZhCN from "./locales/zh-CN.json";
import gatewayEn from "./locales/en.json";
import ssoZhCN from "@alioth/sso-frontend/locales/zh-CN";
import ssoEn from "@alioth/sso-frontend/locales/en";

<I18nProvider
  initialDictionaries={{
    "zh-CN": { ...componentsZhCN, ...gatewayZhCN, ...ssoZhCN },
    en: { ...componentsEn, ...gatewayEn, ...ssoEn },
  }}
>
````

**合并顺序**：宿主自身的键 > 共享包键（后展开覆盖先展开）。如 `login.divider` 在 Gateway 与 SSO 都存在时，以 Gateway 的为准。

#### 8.8.1 审计盲区提示

`.agents/skills/alioth-i18n/scripts/audit-i18n-completeness.ts` 按项目目录独立扫描，**不**追踪跨包 key 引用：

- 宿主 app 引用 `@alioth/sso-frontend/components` 中的 `t("sso.login.divider")` 不被宿主审计器报为「缺失」——因为 SSO 项目的 `i18n-keys.ts` 已记录该键
- 但宿主 app 实际未加载 SSO locales 时，运行时仍会回退为原始 key
- 因此新增跨包消费关系时，必须同时更新宿主的 `main.tsx` / `App.tsx` 来 import 依赖包 locales

#### 8.8.2 检查清单

- [ ] 新增跨包组件库时，`package.json` 的 `exports` 包含 `./locales/{zh-CN,en}`
- [ ] 共享包 locales 键的第一段 = 包名（`sso.*`），不与宿主键冲突
- [ ] 宿主 app 的 `main.tsx` / `App.tsx` 显式 import 并 merge 该包 locales
- [ ] CI 中至少有一个端到端冒烟：宿主 app 加载被消费页面，文本应被翻译（不是原始 key）

---

## 9. 表单验证规范

### 9.1 统一技术栈

| 层          | 技术                                                        |
| ----------- | ----------------------------------------------------------- |
| Schema 定义 | `zod`（`z.object()`）                                       |
| 表单状态    | `react-hook-form` + `@hookform/resolvers/zod`               |
| 表单渲染    | `AutoForm` + `EntityFormPage`（来自 `@alioth/composables`） |

### 9.2 标准骨架

```typescript
import { z } from "zod";
import { EntityFormPage } from "@alioth/composables";
import { useNotification } from "@alioth/components";

const schema = z.object({
  notice: z.string().min(1, "名称不能为空"),
  code: z.string().optional(),
  fk_category: z.coerce.number().optional().nullable(),
  comments: z.string().optional(),
});

type FormValues = z.infer<typeof schema>;

export function EntityFormPage(): React.ReactElement {
  const notify = useNotification();
  const createMutation = entityHooks.useCreate();

  const handleSubmit = (data: FormValues) => {
    createMutation.mutate(data, {
      onSuccess: () => { /* invalidate + navigate */ },
      onError: (err) => notify.apiError(err),
    });
  };

  return (
    <EntityFormPage
      schema={schema}
      onSubmit={handleSubmit}
      fieldConfig={{ /* ... */ }}
    />
  );
}
```

### 9.3 FK 字段类型约定

外键字段统一使用 `z.coerce.number().optional().nullable()`：

```typescript
fk_organization: z.coerce.number().optional().nullable(),
sk_currency: z.coerce.number().optional().nullable(),
```

### 9.4 关联选择器

FK/sk/tk 字段使用 `SearchableSelect` 或 `createAssociationSelect`：

```typescript
import { createAssociationSelect } from '@alioth/components';

const categorySelect = createAssociationSelect({
  hooks: categoryHooks,
  labelField: 'notice',
  valueField: 'id',
});
```

禁止手工拼接 dropdown options、硬编码选项列表。

### 9.5 表单承载容器（三形态）

数据表单 MUST 通过三种规范容器之一承载，禁止手写 fixed-position modal/overlay 实现表单容器：

| 形态                 | 组件                                                       | 适用场景                                                       |
| -------------------- | ---------------------------------------------------------- | -------------------------------------------------------------- |
| **Page（页面）**     | `EntityFormPage`（`@alioth/composables`）                  | 独立业务入口、深度链接（`/new`、`/:id/edit`）、>8 字段完整录入 |
| **Drawer（抽屉）**   | `FormBody`/`AutoForm` + 抽屉容器（`StandardDrawer` 等）    | 详情页内编辑、辅助录入、需保留引用上下文或移动端友好           |
| **Dialog（对话框）** | `FormDialog`（`@alioth/composables`）/ `FormBody` + Dialog | 列表页快捷录入、≤8 字段轻量表单、单实体快速创建                |

- 三形态统一经由 `FormBody`（AutoForm 统一入口）渲染字段，分组/行绑定/`layout`/i18n 规则一致。
- **数据表单禁止使用裸 Modal 承载**——确认/选择等无输入表单的操作仍保留轻量 Modal。

---

## 10. 设计模式与业务语义（前端 v10 新增）

### 10.0 设计 Tokens 体系（唯一来源）

AliothStudio 前端设计 Tokens 的**唯一来源**是 `Framework/frontend/components/tokens.json` 与 `Framework/frontend/components/src/theme-base.css`。
下游应用（Gateway / Meta / 各 Module）通过 CSS 变量继承或覆盖（门禁：`scripts/check/check-design-token-parity.ts`）。

| Token 类别                                        | 来源文件                                                         | 覆盖方式                                                                         |
| ------------------------------------------------- | ---------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| 色彩（primary / accent / muted 等）               | `theme-base.css` `:root` 默认值                                  | 模块通过 `.mod-{name}` 作用域覆盖 `--primary` 等变量                             |
| 暗黑模式中性色（background/foreground/border 等） | `theme-base.css` `.dark` 块                                      | 应用通过 `ThemeProvider` 给 `html` 添加/移除 `.dark`；模块**不**单独覆盖中性变量 |
| 圆角（radius-sm/md/lg/xl/2xl）                    | `theme-base.css` `@theme`                                        | 不推荐覆盖；如需覆盖在应用 `:root` 中修改 `--radius`                             |
| 间距（space-1~24）                                | `theme-base.css` `@theme`                                        | 不可覆盖，强制 4pt 网格                                                          |
| 阴影（shadow-sm~2xl）                             | `theme-base.css` `@theme`                                        | 不可覆盖                                                                         |
| 字体族（sans/Display/Body）                       | `theme-base.css` `@layer base` `@font-face` + body `font-family` | 不可覆盖；模块字体全部走本地自托管（见 §10.0.1）                                 |
| 等宽字体（mono / `font-mono`）                    | `theme-base.css` `@layer base` `@font-face` + tokens.json        | 不可覆盖；统一使用 JetBrains Mono（见 §10.0.1）                                  |
| 布局尺寸（topbar/sidebar/branding 高度宽度）      | `theme-base.css` `@theme`                                        | 不可覆盖                                                                         |

**明暗模式原则**：

- Gateway 与所有 Module 必须支持 **light / dark / system** 三种模式，通过 `@alioth/components` 的 `ThemeProvider` + `ThemeToggle` 实现。
- Meta 为深色优先应用，默认保持暗黑主题；其 `:root` 与 `.dark` 变量保持一致，允许未来扩展浅色主题。**Meta 必须使用 `html:root` / `html.dark` 定义 Bee Theme，避免被后加载的 Framework `theme-base.css` fallback 覆盖。**
- 模块只需在 `.mod-{name}` 中定义 `--primary` 系列变量；深色模式下的背景、文字、边框由 `theme-base.css` `.dark` 统一提供。
- `theme-base.css` 在 `.dark [class^="mod-"]` 中统一提高 `--primary-bg` / `--primary-bg-hover` 的透明度，保证模块主色在深色背景上可见。
- **Cascade 优先级规则**：当应用需要覆盖 `theme-base.css` 的品牌色时，应选择器 specificity 必须高于 `:root` / `.dark`（如 `html:root` / `html.dark`），因为 `@alioth/components` 会在组件导入时自动注入 `theme-base.css`，其同权选择器后加载会覆盖应用级 `:root`/`.dark`。

### 10.0.1 国内访问性与字体本地化

**问题背景**：Google Fonts（`fonts.googleapis.com` / `fonts.gstatic.com`）在**中国大陆地区不可达**（GFW 屏蔽、DNS 污染、连接超时）。任何前端页面若通过 `<link>` 或 `@import` 引用 Google Fonts 资源，在大陆用户访问时将出现 5s+ 的白屏或 FOIT（Flash of Invisible Text），严重劣化首屏体验。

**硬性约束（MUST）**：

1. **生产代码字体一律本地自持**（不引用境外字体 CDN），已知大陆不可达的字体服务包括但不限于：
   - `fonts.googleapis.com` / `fonts.gstatic.com`（Google Fonts）
   - `use.typekit.net`（Adobe Fonts）
   - `fast.fonts.net`（Fonts.com）
   - 任何在大陆可达性不达标的字体服务
2. **所有字体必须从同源（same-origin）加载**，即从当前应用自己的静态资源服务器提供，路径前缀 `/fonts/...`。
3. **HTML 原型与生产代码统一**：HTML 原型（`Pre-Proc/*/Prototypes/*/v*.html`）使用本地 `Framework/frontend/public/fonts/` 下的 woff2 文件（详见 `HTML_DESIGN_SPEC.md §1.2`）；原型产物引用 Google Fonts 等外链由门禁拒绝（门禁：`scripts/check/check-prototype-offline.ts`）。
4. **Vendor 文件策略外推**：`HTML_DESIGN_SPEC.md §1.1` 已规定 React/Babel UMD 走本地 vendor；本条将同一原则外推至字体资源。

**字体规范（2026-06-14 升级）**：

| 用途                        | 字体               | 字重                  | 加载方式                                                                       |
| --------------------------- | ------------------ | --------------------- | ------------------------------------------------------------------------------ |
| Display / Body（Latin）     | **Inter**          | 400 / 500 / 600 / 700 | 本地 woff2（`/fonts/inter-{weight}.woff2`）                                    |
| Mono（Code/IDs/Timestamps） | **JetBrains Mono** | 400 / 500 / 600 / 700 | 本地 woff2（`/fonts/jetbrains-mono-{weight}.woff2`）                           |
| CJK（中文）                 | 系统字体栈         | —                     | `PingFang SC` → `Hiragino Sans GB` → `Microsoft YaHei` → `WenQuanYi Micro Hei` |

**body 字体栈顺序**：

```css
font-family:
  'Inter',
  /* Latin Display/Body, locally hosted */ ui-sans-serif,
  system-ui,
  -apple-system,
  'PingFang SC',
  'Hiragino Sans GB',
  /* macOS / iOS CJK */ 'Microsoft YaHei',
  'WenQuanYi Micro Hei',
  /* Windows / Linux CJK */ sans-serif;
```

**Mono 字体栈顺序**：

```css
font-family:
  'JetBrains Mono',
  /* locally hosted */ ui-monospace,
  SFMono-Regular,
  Menlo,
  Monaco,
  Consolas,
  'Liberation Mono',
  'Courier New',
  monospace;
```

**部署要求**：

- `Framework/frontend/public/fonts/` 目录存放全部 woff2 文件（共 8 个，Inter 4 档 + JetBrains Mono 4 档），总计 ~192KB。
- 主题更新或新增字重时，**必须**同时更新 `tokens.json` / `typography.ts` / `theme-base.css` 三个文件，保持 Token 与代码一致。
- 新增模块若需引入新字体（极少场景），须先在 PR 中同步更新上述三处并附 `docs/specs/MODULE_SPEC.md` ADR。

**违规检测**：

- `alioth-app` 技能的 `audit-html-spec.ts`（`.agents/skills/alioth-app/scripts/audit-html-spec.ts`）会在原型审计中检测 `fonts.googleapis.com` 引用并报错（severity: warning → error）。
- 原型产物遗留旧版 Google Fonts `<link>` 属规约违规，由仓库级门禁检出（门禁：`scripts/check/check-prototype-offline.ts`；与 `HTML_DESIGN_SPEC.md §1.1` 一致）。

### 10.1 模块主题色 `moduleAccent`

> **⚠️ 历史参考**: 以下颜色表按旧版的 L0-L5 分层分配色相。模块当前定位为 Scene/Factor 组合容器，不再代表业务域边界。颜色仅作为视觉区分标志，新场景可自定义色值而不必遵循此表。

模块主题色不再通过 `moduleAccent` prop 传入（已废弃），改为** CSS 变量注入**：

1. `createModuleLayout` 在渲染根元素上自动添加 `mod-{moduleName}` 类名；
2. 模块 `theme.css` 在该类名作用域下定义 `--primary`、`--primary-foreground`、`--primary-bg` 等变量；
3. `ModuleLayout` 内部所有 `bg-primary`、`text-primary`、`bg-primary/10` 等 Tailwind 类自动解析为模块主题色。

在 Gateway 应用视角下，Gateway 的 `MainLayout` 会读取 `useModuleSidebar()` 中的 `accentBarColor`，以内联 `style` 将 `--primary` 注入到模块与外壳的公共祖先上。此时模块 `theme.css` 中的 `.mod-{name} --primary` 被覆盖，**Gateway 外壳与模块内部使用同一主色**，实现 tab 切换时整站同步换色。

> **`accentBarColor` 必须是硬性要求（MUST）**：每个使用 `createModuleLayout` 的模块**必须**在 `getModuleConfig` 中提供 `accentBarColor`（hex 色值）和 `accentBarStyle: "solid"`。若模块未提供 `accentBarColor`，Gateway 外壳无法感知模块主题色，导致 tab 切换时不换色或使用 `:root` fallback，用户无法区分当前所在模块。新增模块的 PR 中缺少 `accentBarColor` 应被拒绝。

**`accentBarStyle` 说明**：

- `"solid"`（推荐）：3px 实色条，使用 `accentBarColor` 的纯色。Gateway 侧边栏 accent bar 始终为 solid 风格。
- `"subtle"`：15% 透明度的 `--primary` 色条，仅用于独立模式（非 Gateway 集成）。

**`accentBarColor` 色表（当前仅列出已存在的模块，新增模块时补充）**：

| 模块              | Hex       | HSL         |
| ----------------- | --------- | ----------- |
| `system-settings` | `#441a97` | 260 70% 35% |

> **历史参考**: 旧架构 24-module 分层色表已移除。当前仅 3 个模块（`system-settings`、`system-approve`、`system-dev`），新增模块时在此表补充色值。
>
> **shop 模块**使用自定义布局（非 `createModuleLayout`），不通过此机制传递 accentBarColor。若 shop 需要 Gateway 主题同步，需在入口处自行调用 `setModuleSidebar()`。

```css
/* Pre-Proc/{ns}/Sources/Apps/Modules/{name}/frontend/src/theme.css */
.mod-orders {
  --primary: 355 68% 48%; /* Crimson Red 绯红 */
  --primary-foreground: 0 0% 100%;
  --primary-hover: 355 68% 40%;
  --primary-bg: 355 68% 48% / 0.08;
  --primary-bg-hover: 355 68% 48% / 0.12;
}

/* 暗黑模式下主色背景透明度由 theme-base.css 统一提升；
   若模块主色在深色背景上对比不足，可在此覆盖。 */
```

`theme-base.css` 在 `:root` 提供默认 fallback 主色（Gateway 品牌蓝），在 `.dark` 提供深色中性色，确保模块独立运行或主题 CSS 未加载时仍有可用颜色。

| 变量名                 | 用途                                   | 必须           |
| ---------------------- | -------------------------------------- | -------------- |
| `--primary`            | 主按钮、活跃状态、品牌图标、强调装饰条 | ✅             |
| `--primary-foreground` | 主色上的文字                           | ✅             |
| `--primary-hover`      | 主按钮悬停                             | 推荐           |
| `--primary-bg`         | 低透明度背景（选中行、图标容器）       | 推荐           |
| `--primary-bg-hover`   | 低透明度背景悬停                       | 推荐           |
| `--accent-foreground`  | accent 上的文字                        | ✅（继承默认） |

### 10.2 统计卡片 `renderStats`

列表页可通过 `renderStats` 在标题与表格之间插入 `StatCard`/`StatGrid` 统计区：

```typescript
renderStats: (
  <StatGrid columns={4}>
    <StatCard label="订单总数" value="356" icon={<FileText className="w-5 h-5" />} />
    <StatCard label="本月新增" value="47" trend={12} icon={<TrendingUp className="w-5 h-5" />} />
    <StatCard label="待处理" value="8" icon={<AlertCircle className="w-5 h-5" />} />
    <StatCard label="总金额" value="¥128.6万" icon={<DollarSign className="w-5 h-5" />} />
  </StatGrid>
),
```

- `value`: 当前静态占位符（后续接入 API 真实数据）
- `trend`: 可选涨跌趋势百分比，自动显示向上/向下箭头
- `icon`: lucide-react 图标组件

### 10.3 详情字段优先级 `detailFields.priority`

详情面板字段按业务重要性分三级，系统字段默认折叠：

| priority    | 含义          | 示例字段                           | 显示策略                           |
| ----------- | ------------- | ---------------------------------- | ---------------------------------- |
| `primary`   | 核心业务标识  | `name`、`code`、`notice`、`number` | 始终显示，排最前                   |
| `secondary` | 辅助/参考信息 | FK 字段、维度字段、金额            | 分组显示在第二区                   |
| `system`    | 系统元数据    | `id`、`created_at`、`updated_at`   | 默认折叠，点击「显示系统字段」展开 |

```typescript
detailFields: [
  { key: "name",  labelKey: "common.name", type: "text", priority: "primary" },
  { key: "code",  labelKey: "common.code", type: "mono", priority: "primary" },
  { key: "fk_subject", labelKey: "common.subject", type: "mono", priority: "secondary" },
  { key: "id",    labelKey: "common.id",    type: "mono", priority: "system" },
  { key: "created_at", labelKey: "common.createdAt", type: "date", priority: "system" },
],
```

### 10.5 状态语义色 `STATUS_COLOR_TOKENS`

`tokens.STATUS_COLOR_TOKENS` 提供 12 种语义状态的 Tailwind class（`Framework/frontend/components/src/tokens/colors.ts`）：

```typescript
import { tokens } from "@alioth/components";

// 直接使用 className:
<span className={tokens.STATUS_COLOR_TOKENS.success.badge}>Active</span>

// badge: 用于 Pill 徽章（bg + text + border）
// dot:   用于状态圆点（bg only）
```

| token                 | 色值       | 语义        |
| --------------------- | ---------- | ----------- |
| `success`             | green      | 成功/已完成 |
| `warning`             | amber      | 警告/待处理 |
| `danger`              | red        | 危险/异常   |
| `info`                | blue       | 信息/进行中 |
| `neutral`             | slate      | 中性/草稿   |
| `active`              | green-800  | 活跃状态    |
| `draft`               | blue-800   | 草稿状态    |
| `locked` / `archived` | slate-500  | 已锁定/归档 |
| `exception`           | red-800    | 异常状态    |
| `occupied` / `free`   | blue/green | 占用/空闲   |

### 10.6 FK 字段选择器 `ReferenceSelect`

表单中的外键字段 MUST 使用 `type: "reference"` 配合已知 endpoint（`type: "number"` 形态由门禁拦截；门禁：`scripts/check/check-block-json.ts`）：

```typescript
fk_country: {
  label: t("common.country"),
  type: "reference",
  endpoint: "/structure/countries",    // API 路径
  labelField: "name",                  // 显示字段
},
```

可用 endpoint 清单（由 `structure` 模块提供）：

| endpoint                 | 返回字段           | 用途         |
| ------------------------ | ------------------ | ------------ |
| `/structure/countries`   | `{id, name}`       | 国家选择     |
| `/structure/currencies`  | `{id, name, code}` | 币种选择     |
| `/structure/units`       | `{id, name, code}` | 单位选择     |
| `/structure/categories`  | `{id, name}`       | 类目选择     |
| `/structure/subjects`    | `{id, name}`       | 主体选择     |
| `/structure/orgs/levels` | `{id, name}`       | 组织层级选择 |
| `/members/users`         | `{id, name}`       | 用户选择     |

`ReferenceSelect` 已实现 30s 缓存，避免重复请求。

**已使用表单**（8 个，16 FK 字段）：SubjectFormPage、VendorFormPage、AddressFormPage、ShippingOrderFormPage、AirliftOrderFormPage、LandOrderFormPage、RailwayOrderFormPage、AssignmentFormPage

### 10.7 表单分组 `groups` + `layout="horizontal"`

字段超过 5 个的表单**必须**分组，启用 horizontal 布局：

```typescript
<EntityFormPage
  groups={[
    { title: t("module.form.basicInfo", {}, { fallback: "基本信息" }), fields: ["name", "code", "public"] },
    { title: t("module.form.relations", {}, { fallback: "关联信息" }), fields: ["fk_subject", "fk_object"] },
  ]}
  layout="horizontal"
/>
```

- `groups` 分割 15+ 字段的长表单为 2-3 个语义区域
- `layout="horizontal"` 使标签和输入在同一行，减少垂直滚动
- i18n key + fallback 确保空数据时仍可读
- 所有 30+ EntityFormPage 表单已应用此模式

### 10.8 行双击导航 `onRowDoubleClick`

所有 `createEntityListPage` 工厂页面自动支持行双击导航到编辑页。实现路径：

1. `InlineEditTable` 的行 `onRowDoubleClick`（工厂 `createEntityListPage` 内建；editable 单元格双击不触发，编辑语义优先）
2. 双击时调用 `handleDetailEdit`（优先级：`renderEditSheet` > `onDetailEditSheet` > `onDetailEdit` > `basePath/{id}`）
3. 不干扰单元格级别的双击编辑

### 10.9 空状态 `EmptyState`

所有实体列表工厂页数据为空时自动显示 `EmptyState` 组件，而非纯文字 `"暂无数据"`：

```
┌──────────────────────────────┐
│         📋 暂无数据           │
│   点击右上角新建按钮创建第一条  │
│      [新建客户]               │
└──────────────────────────────┘
```

- `createEntityListPage` 自动从 `createLabelKey` + `createRoute` 生成空操作按钮
- 可通过 `renderEmpty` 或 `emptyMessage` 覆盖默认行为

### 10.10 数据展示最佳实践

#### 表格列

- ✅ FK 字段必须通过 `_refs` 解析显示名称（`_refs?.subject?.notice`）
- ✅ 刻度字段显示解析值（`_refs?.qk_amount?.mark`）
- ✅ `order_cate` 使用 OrderCateBadge 组件
- ✅ 金额/数值使用 `tabular-nums` 等宽数字

- ✅ 使用 `detailFields` 声明式配置，而非手写 `detailChildren`
- ✅ 系统字段标记 `priority: "system"` 允许默认折叠
- ✅ 时间戳使用 `type: "date"` 自动格式化

#### 表单

- ✅ FK 字段使用 `type: "reference"` + `endpoint`
- ✅ 5+ 字段必须分组 + horizontal
- ✅ 使用 `z.coerce.number()` 处理数字字符串转换
- FK 字段必须使用选择器而非直接输入 ID（`type: "number"` 的 FK 字段禁止）

### 10.11 列表布局选择

| 场景                    | 布局                                                      | 示例         |
| ----------------------- | --------------------------------------------------------- | ------------ |
| 以头像/名称为核心的实体 | 卡片网格 `grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-3` | 人员、组织   |
| 需要对比多列的实体      | 表格 `table`                                              | 订单、任务   |
| 状态驱动的看板          | 看板列                                                    | 审批、任务流 |

### 10.12 行内编辑（Inline Editing）

表格行内快速改值的交互，避免每次修改都打开抽屉表单。接入路径（二选一）：

1. **整表方案**：`InlineEditTable`（`@alioth/components`，基于 `@tanstack/react-table`）——适合新表格。列定义 `editable: true` 声明可编辑列；受控编辑态通过 `editingRowId` / `editingColumnId` / `editValues` + `onCellClick` / `onCellChange` / `onCellCommit` / `onCellCancel` 回调驱动。键盘语义：Enter 提交、Esc 取消、失焦提交。
   > **现状（2026-08）**：已从 barrel 导出，但无业务消费方、无单元测试。整表迁移注意：其 `ui/table` 渲染无 colgroup 透传、边框模型与模块自绘 `.data-table` 不同，**既有手写表格勿直接迁移**（会回归列宽/边框样式）。
2. **手写表格内嵌**（既有 `.data-table` 表格的渐进路径，WZ `TasksPage` 现行范式）：行解锁 → 可编辑列**用 `InlineEditCell` 组件按类型渲染**（select/date/text/number/multi-select 五种类型，键盘语义 Enter 提交 / Esc 取消统一封装）→ 保存/取消。**失焦不提交、不重置编辑态**（防点「保存」时 input 失焦导致保存点击丢失）。
   > **`InlineEditCell` 现状（2026-08）**：`@alioth/components` 通用 cell 编辑原语，WZ 已消费（8 列类型化行编辑），配 7 个单元测试（交互契约 + 类型渲染）。新模块手写表格行编辑**优先复用**，勿重复实现输入类型分发。

**可编辑字段约束（实测）**：业务字段（cargo/origin/dest/mode/pickupTime/eta/carrier/cost 等）经 `comments` JSON 通道提交与读回（`updateMutation` 序列化），**后端无需 DTO 扩展**。

**提交语义**：无变更保存不发请求（直接退出编辑态）；提交失败保持编辑态 + 错误提示，保留用户输入。

### 10.13 详情页 vs 抽屉

| 场景                                 | 推荐方案                  |
| ------------------------------------ | ------------------------- |
| 详情内容少（< 5 字段），快速查看     | 抽屉（Drawer，view 模式） |
| 详情内容多（关联数据/子列表/时间线） | 独立页面（DetailPage）    |
| 需要深度链接到该详情的场景           | 独立页面（DetailPage）    |

### 10.14 颜色字段色表复用

**禁止自建色表。** 对于 `t_color_` 等颜色字段，使用统一的颜色选项列表：

```typescript
const COLORS = [
  { value: 'blue', label: '蓝色', color: '#3B82F6' },
  { value: 'green', label: '绿色', color: '#10B981' },
  { value: 'purple', label: '紫色', color: '#8B5CF6' },
  { value: 'orange', label: '橙色', color: '#F59E0B' },
  { value: 'red', label: '红色', color: '#EF4444' },
  { value: 'cyan', label: '青色', color: '#06B6D4' },
  { value: 'pink', label: '粉色', color: '#EC4899' },
];
```

> ⚠️ **LEGACY NOTICE — MODULE Backend Mapping Obsolete**
> §11 describes the Module frontend directory structure and architecture, which applies while Modules remain as deployment units. However, **the backend counterpart (Modules/{name}/backend/) has been fully migrated to Service/Scene architecture** (`Pre-Proc/{namespace}/Sources/Apps/Services/{name}/backend/`).
>
> - **What still applies**: `ModuleLayout` 工厂和 accent color 机制（§10.1 的 3-module 色表）适用于现有模块前端。模块前端（`Pre-Proc/{ns}/Sources/Apps/Modules/*/frontend/`）仍作为部署产物存在。
> - **What is obsolete**: Any reference to module backends as the backend implementation strategy. Section 11's architecture assumes modules provide both frontend and backend — this backend side is no longer the current standard.
> - **Current architecture**: Services (`Pre-Proc/{namespace}/Sources/Apps/Services/{name}/service.json`) are the atomic backend units, Scenes compose them into flows, and Modules persist as frontend deployment coordinates.

## 11. 模块前端目录结构与架构规范

所有 Module 前端统一遵循以下约定。**例外**：`shop`（双区域架构，见 §11.9，不使用 ModuleLayout 见 §11.11.6），其余模块均需对齐。

### 11.1 最小目录结构

```
Pre-Proc/{ns}/Sources/Apps/Modules/{name}/frontend/
├── package.json              # 包名 @alioth/{name}-frontend
├── vite.config.ts
├── vitest.config.ts
├── tsconfig.json
├── postcss.config.js
├── index.html
└── src/
    ├── main.tsx              # Vite 入口（dev 模式使用）
    ├── single-spa.tsx        # single-spa 微前端生命周期（生产模式使用）
    ├── App.tsx               # 根组件：路由 + ModuleI18nShell + 字典合并
    ├── theme.css             # 模块主题色 CSS 变量
    ├── index.css             # Tailwind 指令入口
    │
    ├── api/                  # API 客户端 + hooks
    │   └── {entity}.ts       # 每实体一个文件
    │
    ├── pages/                # 页面级组件（按实体命名，扁平结构）
    │   ├── {Entity}ListPage.tsx
    │   └── {Entity}FormPage.tsx
    │
    ├── components/           # 模块私有 UI 组件
    │   ├── {Module}Layout.tsx    # 路由布局容器
    │   └── forms/                # 复杂表单子组件
    │
    ├── stores/               # Jotai v2 atom 定义
    │   └── {entity}_store.ts     # 每实体一个文件
    │
    ├── exports/              # 跨模块共享出口
    │   └── index.ts
    │
    └── locales/              # 国际化字典（双语）
        ├── zh-CN.json
        └── en.json
```

#### 强制项

- `api/` 目录必须有对应实体的 hooks 文件，禁止在 `pages/` 组件内联定义查询逻辑
- `pages/` 目录必须有至少一个入口页面
- `components/` 目录必须存在（可仅包含 Layout 组件）
- `exports/` 目录必须存在（可仅导出空对象 `{}`）
- `locales/` 必须提供中英双语字典

#### 规范

- 目录结构统一使用 `stores/` 管理状态，不同时存在 `stores/` 和 `atoms/`（`atoms/` 与 `src/utils/`、`src/lib/`、`src/types/`、`src/hooks/` 目录由门禁拒绝：`scripts/check/check-frontend-layer-rules.ts`）
- `src/` 根目录下只保留 §11.1 最小目录集（`types/`、`hooks/` 不在最小目录集内，见 §11.4 组件分层）
- `@alioth/*` 的引用必须从顶层导出导入，不得使用深路径 import

### 11.2 必选框架依赖

每个模块前端 `package.json` 的 `dependencies` 必须包含以下 workspace 包：

```json
{
  "dependencies": {
    "@alioth/api": "workspace:*",
    "@alioth/components": "workspace:*",
    "@alioth/hooks": "workspace:*",
    "@alioth/i18n": "workspace:*",
    "@alioth/utils": "workspace:*",
    "@alioth/types": "workspace:*"
  },
  "devDependencies": {
    "@alioth/config": "workspace:*",
    "@alioth/testing": "workspace:*"
  }
}
```

#### 安装原则

- 全部通过 `workspace:*` 引用（门禁：`scripts/check/check-workspace-pin-consistency.ts`）
- 框架已封装的底层依赖（`@radix-ui/*`、`@hookform/resolvers`、`recharts`、`sonner`、`vaul` 等）由 `@alioth/components` 统一管理并提供（门禁：`scripts/check/check-pnpm-circular-deps.ts`）
- 第三方运行时依赖（`react-router`、`zod`、`lucide-react` 等）必须与 Framework peerDependencies 版本对齐

#### 框架内部包分层约束（2026-06-13）

`Framework/frontend/` 下 `@alioth/{api,hooks,components}` 三包的 workspace 依赖**必须保持单向分层**（门禁：`scripts/check/check-workspace-deps.sh`）。

**当前依赖结构**（源码已验证）：

| 包           | workspace 依赖                                                  | 关键 npm 依赖                            |
| ------------ | --------------------------------------------------------------- | ---------------------------------------- |
| `api`        | `@alioth/i18n`、`@alioth/types`                                 | `next-themes`、`@tanstack/react-query`   |
| `hooks`      | `@alioth/api`、`@alioth/types`                                  | `jotai`、`react-hook-form`、`rxjs`       |
| `components` | `@alioth/api`、`@alioth/hooks`、`@alioth/i18n`、`@alioth/utils` | `@radix-ui/*` × 24、`next-themes`、`zod` |

**规则**：

| 方向                        | 允许？   | 原因                                                                                                          |
| --------------------------- | -------- | ------------------------------------------------------------------------------------------------------------- |
| `api → {components, hooks}` | **禁止** | API 层不应引用 UI 或 hooks 层状态逻辑。需 `ThemeProvider` 等运行时组件时，从 `next-themes` 等底层库直接引入。 |
| `hooks → components`        | **禁止** | hooks 层是纯状态/atom 逻辑，不涉及 UI。                                                                       |
| `components → {api, hooks}` | 允许     | UI 层可引用 HTTP 客户端和状态 hooks。                                                                         |

**防线**：新增 `workspace:*` 依赖后必须执行 `pnpm install`，确认无 `[WARN] There are cyclic workspace dependencies` 输出。

> **2026-06-13 复盘**：`api` 通过 `bootstrap.tsx` 引入 `components` 的 `ThemeProvider` → `api → components → {hooks, api}` 形成三点循环。修复：`api/package.json` 移除 `@alioth/components` 依赖，改用 `next-themes` 直接导入。`pnpm install` 从 WARN 变为 0 警告。教训：任一方向新增 `workspace:*` 依赖前，先检查是否会在图中引入反向边。

### 11.3 模块入口约定

#### App.tsx — 根组件模板

```tsx
import { Routes, Route, Navigate } from 'react-router';
import { ModuleI18nShell } from '@alioth/i18n';
import { componentsZhCN, componentsEn } from '@alioth/components';
import zhCNDict from './locales/zh-CN.json';
import enDict from './locales/en.json';
import { ModuleLayout } from './components/{Module}Layout';
import './theme.css';
import './index.css';

export default function App() {
  return (
    <ModuleI18nShell
      dictionaries={{
        'zh-CN': { ...componentsZhCN, ...zhCNDict },
        en: { ...componentsEn, ...enDict },
      }}
    >
      <Routes>
        <Route element={<ModuleLayout />}>
          <Route index element={<Navigate to="/entity" replace />} />
          <Route path="/entity" element={<EntityListPage />} />
          <Route path="/entity/new" element={<EntityFormPage />} />
          <Route path="/entity/:id/edit" element={<EntityFormPage />} />
        </Route>
      </Routes>
    </ModuleI18nShell>
  );
}
```

**关键规则**：

- 字典合并：组件字典先展开，模块字典后展开，模块键有更高优先级（§8.5）
- 根路径跳转到首个实体列表页，不设 Dashboard 总览页面（AGENTS.md 已约定）
- 禁止在 App.tsx 中定义超过 20 行的辅助函数或内联组件——应抽取到 `components/` 或 `api/`

#### single-spa.tsx — 微前端生命周期

```tsx
import { createMicroAppLifecycle } from '@alioth/api';
import App from './App';
import { BrowserRouter } from 'react-router';

export const { bootstrap, mount, unmount } = createMicroAppLifecycle({
  moduleName: '{module_name}',
  App,
  renderApp: (app, props) => (
    <BrowserRouter basename={props.baseUrl as string}>{app}</BrowserRouter>
  ),
});
```

**关键规则**：

- `moduleName` 必须与 pnpm workspace 包名后缀一致（如 `orders` → `@alioth/orders-frontend`）
- `BrowserRouter` 的 `basename` 必须从 `props.baseUrl` 取用，禁止硬编码

### 11.4 组件分层规则

| 层级            | 目录                                | 职责                                             | 使用方式                                                    |
| --------------- | ----------------------------------- | ------------------------------------------------ | ----------------------------------------------------------- |
| **L0 框架组件** | `@alioth/components`                | 通用 UI 原子、CRUD 工厂、布局外壳                | 所有模块直接引用，禁止覆写                                  |
| **L1 模块布局** | `src/components/{Module}Layout.tsx` | 路由容器、模块侧栏 Tabs、Suspense 边界           | App.tsx 中挂载一次                                          |
| **L2 页面组件** | `src/pages/`                        | 完整页面（工厂列表页/表单页）                    | 一个页面一个文件，禁止页面文件内定义超过 100 行的内联子组件 |
| **L3 私有组件** | `src/components/`                   | 模块特有的子组件（复杂选择器、自定义可视化块等） | 优先用框架组件组合实现；确需自定义才新增                    |
| **L4 共享组件** | `src/exports/`                      | 跨模块可复用的组件/类型                          | 显式 export，在 package.json `exports` 字段注册             |

#### 组件下沉原则

- 无业务逻辑的纯展示组件，优先用框架 `@alioth/components` 的现有组件组合
- 在其他 2 个以上模块中重复出现的组件，应提升到 `Framework/frontend/components/src/` 作为共享组件
- 禁止在 `src/pages/` 目录下创建子目录组织代码（**例外**：见 §11.9 shop 模块）

### 11.5 模块间组件引用

跨模块前端引用通过 pnpm workspace 显式声明依赖实现：

```json
// 引用方 Pre-Proc/{ns}/Sources/Apps/Modules/{other}/frontend/package.json
{
  "dependencies": {
    "@alioth/orders-frontend": "workspace:*"
  }
}
```

**约束**：

- 只能引用被引用模块 `exports/` 目录中显式导出的内容
- 禁止引用被引用模块内部 `pages/`、`stores/`、`components/`（非 `exports/`）的组件
- 跨模块前端依赖关系必须与后端模块依赖关系一致，遵守 Alioth 交换本体分层模型（低层不可依赖高层）

### 11.6 测试约定

每个模块前端必须包含基础测试：

```
src/pages/
├── {Entity}ListPage.tsx
├── {Entity}ListPage.test.tsx     # 页面级渲染测试
├── {Entity}FormPage.tsx
└── {Entity}FormPage.test.tsx     # 表单级渲染测试
```

- 纯函数（schema 定义、工具函数）使用 `vitest` 直接测试
- UI 组件使用 `@testing-library/react` 渲染测试
- 使用 `@alioth/testing` 提供的辅助工具
- 测试文件命名 `{被测文件}.test.tsx`，与源文件同目录

### 11.7 禁用项汇总

| 类别     | 禁用                                                                | 原因                                                                |
| -------- | ------------------------------------------------------------------- | ------------------------------------------------------------------- |
| 包管理   | 自装 `@radix-ui/*`、`react-hook-form`、`recharts`、`sonner`、`vaul`（门禁：`scripts/check/check-pnpm-circular-deps.ts`） | 框架已封装，应通过 `@alioth/components` 使用                        |
| 状态管理 | Zustand、Redux、Recoil（门禁：`scripts/check/check-state-management-libs.ts`） | 已强制 Jotai v2 + React Query                                       |
| 目录结构 | 在 `src/` 下创建 `hooks/`、`utils/`、`lib/`、`types/`、`atoms/`（由门禁拒绝：`scripts/check/check-frontend-layer-rules.ts`） | 统一 `api/`（hooks）+ `components/`（私有组件）+ `stores/`（atoms） |
| 入口     | App.tsx 之外再创建第二份路由定义                                    | 单一路由来源，禁止分散                                              |
| 字典     | 裸 `import 'react-i18next'` 或自定义 i18n 实例                      | 必须使用 `@alioth/i18n` 的 `ModuleI18nShell`                        |
| 组件命名 | 文件名与框架组件同名（如自建 `Button.tsx`）                         | 冲突风险，应基于框架组件封装并改名                                  |

### 11.8 全量模块对照表

| 模块          | 有前端     | 对齐模板    | 说明               |
| ------------- | ---------- | ----------- | ------------------ |
| `clients`     | ✅         | ✅          |                    |
| `vendors`     | ✅         | ✅          |                    |
| `product`     | ✅         | ✅          |                    |
| `inventory`   | ✅         | ✅          |                    |
| `orders`      | ✅         | ✅          |                    |
| `finance`     | ✅         | ✅          |                    |
| `process`     | ✅         | ✅          |                    |
| `logistics`   | ✅         | ✅          |                    |
| `transport`   | ✅         | ✅          |                    |
| `channel`     | ✅         | ✅          |                    |
| `demand`      | ✅         | ✅          |                    |
| `criterion`   | ✅         | ✅          |                    |
| `plan`        | ✅         | ✅          |                    |
| `develop`     | ✅         | ✅          |                    |
| `structure`   | ✅         | ✅          |                    |
| `measurement` | ✅         | ✅          |                    |
| `devtools`    | ✅         | ✅          |                    |
| `access`      | ✅         | ✅          |                    |
| `members`     | ✅         | ✅          |                    |
| `approve`     | ✅         | ✅          |                    |
| `ai`          | ✅         | ✅          |                    |
| `devices`     | ✅         | ✅          |                    |
| **`shop`**    | ✅         | **⚠️ 例外** | 见 §11.9           |
| `contacts`    | 仅 backend | —           | 仅 backend，无前端 |

### 11.9 例外：`shop` 模块

`shop` 具有**双区域架构**——同时包含门店前台（storefront）和后台管理（admin），不适用标准模块的扁平页面结构。

#### 差异点

| 维度     | 标准模块                                               | shop 模块                                                                                     |
| -------- | ------------------------------------------------------ | --------------------------------------------------------------------------------------------- |
| 页面结构 | 扁平：`pages/{Entity}ListPage.tsx`                     | 垂直：`pages/store/{area}/{Page}.tsx` + `pages/admin/{Page}.tsx`                              |
| 页面模式 | CRUD 工厂（`createEntityListPage` / `EntityFormPage`） | admin 侧用 CRUD 工厂；store 侧用自定义页面（`ProductDetailPage`、`CartPage`、`CheckoutPage`） |
| 状态管理 | 实体级 Jotai stores                                    | 标准实体 stores + 独有 `cartStore.ts`（客户端购物车状态）                                     |
| 路由层级 | 单层 `/entity`                                         | 双层 `/store/products`、`/store/cart`、`/admin/products`                                      |
| 适用场景 | 后台数据管理                                           | B2C/B2B 门店前台 + 后台管理                                                                   |

#### shop 被允许豁免的规则

- ✅ `pages/` 下可以使用子目录（`store/`、`admin/`）
- ✅ 可以不使用 CRUD 工厂构建 store 侧页面
- 可以定义模块特有的客户端状态（如购物车 `cartStore`）
- shop 仍需遵守：入口模式（single-spa.tsx）、字典合并、框架依赖声明、`exports/` 目录、i18n 键命名规范
- admin 侧页面仍应尽量使用 CRUD 工厂

> 此例外仅限 `shop` 模块。新增模块如需类似双区域架构，需先讨论决定是否扩展此规范。

### 11.10 页面宽度约束

所有列表/详情页面必须充分利用窗口宽度：

```css
/* 页面容器 */
width: 100%;
max-width: none; /* 禁止固定宽度限制 */
```

- 禁止使用 `max-width: 960px`、`max-width: 1200px` 或类似数值限制
- 内容区宽度由 ModuleLayout 侧边栏自然约束
- 表格列宽使用 `min-width` + `max-width` 灵活控制，禁止固定像素宽度

### 11.11 ModuleLayout 统一布局（除 shop 外均须使用）

所有模块（`shop` 除外）必须使用 `@alioth/components` 的 **`ModuleLayout`** 组件作为路由布局容器，由 **`createModuleLayout`** 工厂生成。

> 详细实现参考：`Framework/frontend/examples/module-template/LAYOUT_REFERENCE.md`

#### 11.11.1 工厂模式

```tsx
// components/{Module}Layout.tsx
import { createModuleLayout } from "@alioth/composables";
import { useT } from "@alioth/i18n";
import type { MainNavItem } from "@alioth/components";

function useNavItems(t: ReturnType<typeof useT>): MainNavItem[] {
  return [
    { id: "entity1", label: t("module.entity1"), icon: "FileText", href: "/entity1" },
    { id: "entity2", label: t("module.entity2"), icon: "List", href: "/entity2" },
  ];
}

export const {Module}Layout = createModuleLayout({
  moduleName: "{module}",                            // 模块标识，用于 i18n key 前缀和 theme accent
  getModuleConfig: (t) => ({
    title: "{Module}",
    subtitle: t("{module}.module.title"),
    icon: "{IconName}",                              // lucide-react icon 名
  }),
  useNavItems,
});
```

#### 11.11.2 布局架构

`ModuleLayout` 封装完整的布局外壳：

```
ModuleLayout (flex h-screen)
├── Sidebar (w-60, 可折叠 → w-16)
│   ├── MainNav (导航项, 由 useNavItems 提供)
│   │   └── 当前激活项高亮（自动匹配 location.pathname）
│   └── SideFoot (折叠/展开按钮)
├── ContentArea (flex-1)
│   └── <Outlet />  ← 子路由页面在此渲染
└── WorkspaceDock (右侧面板，按需出现)
```

- Sidebar 可折叠：展开 240px，折叠 64px（仅显示 icon）
- 导航激活项通过 `useLocation().pathname` 自动推断（`id` 前缀匹配 `href`）
- 支持通过 `aiContext` 配置注入 EmpAgent 上下文
- Gateway 应用视角下：模块 ModuleLayout 全屏渲染（含 Sidebar + TopBar + 内容区），作为 Gateway MainLayout `<Outlet />` 的内容。`embedded: true` 仅用于 Meta 模块预览等独立嵌入场景，此时 Sidebar + TopBar 隐藏，仅渲染 ContentArea。
- Sidebar 顶部不渲染品牌 block。品牌由 Gateway TopBar 统一管理（详见 §11.11.6）。
- SideFoot 仅有折叠/展开切换功能，不含用户档案、退出登录等其他操作入口

#### 11.11.3 Sidebar 尺寸与折叠按钮 Token

`ModuleLayout` 的 Sidebar 是跨模块视觉一致性的核心区域。以下 Token 同时约束 React 组件实现与 HTML 原型设计，所有模块必须遵守。

| 组件                                 | 尺寸 / 样式                                                                                                                                | Tailwind 等价                                                                             | 说明                                   |
| ------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------- | -------------------------------------- |
| `.gl-sidebar` 展开宽度               | 240px                                                                                                                                      | `w-60`                                                                                    | Sidebar 展开态固定宽度                 |
| `.gl-sidebar.collapsed` 折叠宽度     | 64px                                                                                                                                       | `w-16`                                                                                    | 仅显示 icon 的紧凑态                   |
| `.gl-branding` 高度                  | 56px                                                                                                                                       | `h-14`                                                                                    | 模块品牌标识区高度，与 TopBar 高度一致 |
| `.gl-branding` 内间距                | `gap: 10px; padding: 0 16px`                                                                                                               | `gap-2.5 px-4`                                                                            | icon 与标题间距                        |
| `.gl-branding .icon`                 | 40×40px，圆角 12px                                                                                                                         | `h-10 w-10 rounded-xl`                                                                    | 模块品牌 icon 容器                     |
| `.gl-branding .icon svg`             | 22×22px                                                                                                                                    | `w-[22px] h-[22px]`（arbitrary）                                                          | 模块品牌 icon                          |
| `.gl-nav-group-title`                | `font-size: 10px; font-weight: 700; uppercase; letter-spacing: 0.06em; color: hsl(var(--muted-foreground) / 0.55); padding: 14px 16px 4px` | `text-[10px] font-bold uppercase tracking-wide text-muted-foreground/55 py-3.5 px-4 pb-1` | 导航分组标题                           |
| `.gl-nav-item`                       | `font-size: 14px; font-weight: 500; padding: 8px 16px; margin: 0 8px; border-radius: 6px`                                                  | `text-sm font-medium py-2 px-4 mx-2 rounded-md`                                           | 导航项                                 |
| `.gl-sidebar.collapsed .gl-nav-item` | 36×36px，居中，margin 2px auto                                                                                                             | `h-9 w-9 mx-auto my-0.5`                                                                  | 折叠态导航项，仅显示 icon              |
| `.gl-collapse-btn`                   | 28×28px，圆角 6px                                                                                                                          | `h-7 w-7 rounded-md`                                                                      | Sidebar 折叠/展开按钮                  |
| `.gl-collapse-btn svg`               | 14×14px，`stroke-width: 2`                                                                                                                 | `w-3.5 h-3.5`（arbitrary）                                                                | 折叠按钮图标                           |

**折叠按钮图标语义（必须遵守）**：

- Sidebar **展开**状态显示 `PanelLeft`（表示左侧面板正在展示，点击收起）
- Sidebar **折叠**状态显示 `PanelRight`（表示面板已收起在左侧，点击展开）
- **禁止**使用 hamburger 图标、`ChevronLeft/ChevronRight`、双向箭头替代
- Gateway 视角的 Navigation 侧栏折叠按钮必须保持同一套 `PanelLeft` / `PanelRight` 方向语义

> **HTML 原型对齐**：HTML 原型阶段的具体 CSS 书写规范见 `docs/specs/HTML_DESIGN_SPEC.md` §3.1 / §3.2；生产 React 实现以本节 Token 为准。

#### 11.11.4 导航项规范

| 属性       | 类型                        | 说明                                                          |
| ---------- | --------------------------- | ------------------------------------------------------------- |
| `id`       | `string`                    | 导航项标识，用于 active 匹配                                  |
| `label`    | `string`                    | 显示文本，已通过 `t()` 翻译                                   |
| `icon`     | `string`                    | lucide-react icon 名                                          |
| `href`     | `string`                    | 路由路径，相对于模块基准路径                                  |
| `badge`    | `string \| undefined`       | 徽标数字或文本                                                |
| `children` | `SubNavItem[] \| undefined` | 子导航（二级导航）                                            |
| `section`  | `string`                    | 分组标题（blockAssembly 模式下由 `blockNavKeys.groups` 派生） |

##### 11.11.4.1 blockAssembly 导航分组规则（`deriveNavItems`）

当使用 `createModuleLayout` + `blockAssembly` 模式时，导航项由 `deriveNavItems` 从 `module.json` 自动派生。
分组聚合由 `MainNav.groupNavItems` 完成，按 `section` 字符串相等性合并**连续**项。

```typescript
// createBlockRoutes.tsx — deriveNavItems 核心逻辑
blocks
  .slice()
  .sort((a, b) => {
    // 先按 group 在 navigation.groups 数组中的索引排序
    const groupIdxA = navigation.groups.findIndex((g) => g.id === a.group);
    const groupIdxB = navigation.groups.findIndex((g) => g.id === b.group);
    if (groupIdxA !== groupIdxB) return groupIdxA - groupIdxB;
    // 同一 group 内按 order 排序
    return a.order - b.order;
  })
  .map((s) => ({
    id: s.id,
    label: t(keyMap.labels[s.id] || s.id),
    section: t(keyMap.groups[s.group] || s.group),
    href: `/${s.id}`,
    icon: s.icon || navigation.groups.find((g) => g.id === s.group)?.icon || 'FileText',
  }));
```

**关键约束：**

1. **导航组顺序由 `navigation.groups[]` 数组决定** — 第一个 group 的 blocks 出现在导航最上方。
2. **每个 block 的 `group` 值必须存在于 `navigation.groups[].id` 中** — 否则 `deriveNavItems` 丢失 group icon fallback。
3. **`blockNavKeys.groups` 必须为每个 group id 提供 i18n 映射** — 缺失时 `t()` 返回 raw group id 字符串，不拆散分组结构但文案不可读（参见 2026-07-24 system-dev 修复）。
4. **排序必须先按 group 在 `navigation.groups` 数组中的位置，再按组内 `order`** — `MainNav.groupNavItems` 只合并连续相同 section 的项。仅按 `order` 排序会导致交叉排列、分组失效。
5. **`blockNavKeys.labels` 必须覆盖所有 block id** — 否则 `t()` 返回 raw block id。

关联实现：`Framework/frontend/composables/src/block/createBlockRoutes.tsx` `deriveNavItems`；
`MainNav.groupNavItems`：`Framework/frontend/components/src/components/layout/MainNav.tsx`。

##### 11.11.4.2 blockNavKeys 接口契约

当 `createModuleLayout` 收到 `blockAssembly` 配置时，必须同时提供 `blockNavKeys`：

```typescript
interface BlockNavKeyMap {
  labels: Record<string, string>; // block id → i18n key
  groups: Record<string, string>; // navigation.groups[].id → i18n key
}
```

**必填约束：**

| 约束                                           | 说明                                              | 缺失后果                    |
| ---------------------------------------------- | ------------------------------------------------- | --------------------------- |
| `labels` 覆盖所有 block id                     | 每个 `blockAssembly.blocks[].id` 必须对应一个 key | block label 显示为 raw id   |
| `groups` 覆盖所有 group id                     | 每个 `navigation.groups[].id` 必须对应一个 key    | group section 显示为 raw id |
| 每个 i18n key 必须存在于 zh-CN.json 和 en.json |                                                   | 翻译 fallback 后显示 key 名 |
| 删除 group id 时同步删除对应的 i18n key        |                                                   | 留下死键                    |

##### 11.11.4.3 serviceBindings 契约

`blockAssembly.serviceBindings` 声明每个 block 依赖的 service collection。binding 标识符的解析方式取决于 namespace 约定：

```json
{
  "serviceBindings": {
    "{block-id}": { "services": ["{binding-id-1}", "{binding-id-2}"] }
  }
}
```

| Namespace   | binding-id 解析方式                                                       | 示例                           |
| ----------- | ------------------------------------------------------------------------- | ------------------------------ |
| AVIC-CAASEC | 直接对应 `Sources/Apps/Services/{id}/service.json` 的 service id          | `"orchestration"`、`"monitor"` |
| WZ          | 使用 factor code（如 `FJA`），由 namespace ontology/service metadata 解析 | `"FJA"`                        |
| Alioth      | 同 AVIC-CAASEC，直接对应 service id                                       | `"commitment"`、`"identity"`   |

**约束：**

1. 每个 `blockAssembly.blocks[].id` 应有对应 binding 条目（缺失不报错但运行时缺少 service 引用）
2. binding-id 在同一 namespace 内解析方式一致（不混合 service id 与 factor code；门禁：`scripts/check/check-ontology-coords.ts`）
3. 跨 NS 不互通

##### 11.11.4.4 layer 语义

`module.json` 的 `layer` 字段标识模块在依赖拓扑中的层级（Rust 类型 `i32`，项目 UI 展示 L0–L5 范围）：

| 值   | 含义                               | 示例                            |
| ---- | ---------------------------------- | ------------------------------- |
| `0`  | 默认值。无特殊依赖约束的底层模块。 | system-settings, org-management |
| `1`  | 依赖 layer 0 模块的业务模块        | system-dev, transport-wz        |
| `2+` | 更高层级（按业务复杂度递增）       | 目前无实例                      |
| 缺失 | 默认为 0                           | 兼容旧模块                      |

**约束**：依赖目标模块的 `layer` 值 MUST 不高于调用方（`Meta/backend/src/module_topology.rs` 拒绝；门禁：`scripts/check/check-module-contract.mjs`）。同层依赖 SHOULD NOT，建议通过事件解耦（`module_validator.rs` 返回 Warning，不阻断）。

##### 11.11.4.5 块内子路由（splat 契约）

`createBlockRoutes` 为每个块生成的路由 path MUST 带 splat（`{blockId}/*`）：

| 项         | 规约                                                                              |
| ---------- | --------------------------------------------------------------------------------- |
| 生成形态   | 块路由 path 末尾恒追加 splat：`{href 去前导斜杠}/*`（落地路径空 splat 与深层路径同表） |
| 原因       | 块入口组件常以内嵌 `<Routes>` 承载**块内子路由**（列表 → 详情/表单，如 `/pipeline-list/:id`）；父路由无 splat 时 React Router 不匹配深层路径，请求落到 `path="*"` 兜底并**静默退回块落地页**，子路由永不可达 |
| 落地页     | `index` 仍由块入口的内嵌 `<Routes index>` 承载；`/{blockId}` 与 `/{blockId}/…` 均落在同一块条目下 |
| 未知子路径 | 由块入口内嵌 `<Routes path="*">` 退回块落地页（不是外层兜底跳首块）              |

实测判据（父 `path="x"`）：`/x/42` → 外层兜底；父 `path="x/*"`：`/x/42` → 子路由、`/x` → 落地 index。回归测试见 `Framework/frontend/composables/src/block/createBlockRoutes.test.ts`（`describe('createBlockRoutes 匹配语义')`）。

#### 11.11.5 主题色（moduleAccent）

`createModuleLayout` 在渲染根元素上自动添加 `mod-{moduleName}` 类名，模块主题色通过该类名下的 CSS 变量注入 `ModuleLayout` 作用域。

**`accentBarColor` 必须配置（MUST）**：每个 `createModuleLayout` 调用的 `getModuleConfig` **必须**包含 `accentBarColor`（hex 色值）和 `accentBarStyle: "solid"`，详见 §10.1 色表。这是 Gateway 应用视角下模块切换时整站同步换色的必要条件。`scripts/check/check-module-accent.sh` 在 CI 中会自动验证。

**变量来源链（按优先级从高到低）**：

1. Gateway 应用视角注入：Gateway `MainLayout` 读取 `useModuleSidebar().branding.accentBarColor`，以内联 `style` 注入 `--primary`，使外壳与模块同色。
2. 模块覆盖：`Pre-Proc/{ns}/Sources/Apps/Modules/{name}/frontend/src/theme.css` 的 `.mod-{name}` 作用域。
3. 应用全局覆盖：Gateway / Meta 在自身 `:root` 中定义全局主色（仅影响应用级 UI 和非应用视角页面）。
4. 默认 fallback：`Framework/frontend/components/src/theme-base.css` 的 `:root`（Gateway 品牌蓝）。

**必须定义的模块变量**：

| 变量                   | 示例值               | 说明                            |
| ---------------------- | -------------------- | ------------------------------- |
| `--primary`            | `355 68% 48%`        | HSL 主色                        |
| `--primary-foreground` | `0 0% 100%`          | 主色上文字                      |
| `--primary-bg`         | `355 68% 48% / 0.08` | 低透明背景，用于选中行/图标容器 |

**推荐定义的模块变量**：

| 变量                 | 说明           |
| -------------------- | -------------- |
| `--primary-hover`    | 主按钮悬停     |
| `--primary-bg-hover` | 低透明背景悬停 |

模块 `theme.css` 必须 `@import "@alioth/components/theme-base.css";` 并添加 `@source "../../../../Framework/frontend/components/src";`，确保 Tailwind v4 能扫描到组件类名。

#### 11.11.6 品牌标识唯一性（应用视角下 Sidebar 禁显模块品牌）

Gateway 在 **App 视角**（`useAppContext().isAppPerspective === true`）下，顶部 `TopBar` 已在 `logo` 槽展示 **应用名**（`currentApp.name`，如「运输管理系统」）作为品牌标识。模块 Sidebar 顶部品牌 block **已整体移除**（`AppBrandHeader`/`GatewayBrandHeader` 组件已删除），避免「TopBar 品牌 + Sidebar 模块品牌」双重出现。

**实现状态**：`ModuleLayout` 仍通过 `setModuleSidebar()` 将 `title`/`subtitle`/`icon`/`accentBarColor` 推送给 Gateway。`accentBarColor` 由 Gateway `AppContentArea` 用于同步 `--primary` CSS 变量实现整站换色。品牌 block 的视觉元素（icon + title + subtitle + 3px accent bar）不再在 Sidebar 中渲染。

**模块规约（MUST）**：每个使用 `createModuleLayout` 的模块在 `getModuleConfig` 中**建议**设置 `hideBrand: true`（字段保留作为约定声明，但不影响视觉渲染）。`title` / `subtitle` / `icon` 必须保留（用于 setModuleSidebar 推送 → TopBar 同步 + SEO + 模块元数据）。

```ts
// Pre-Proc/{ns}/Sources/Apps/Modules/{name}/frontend/src/components/{Name}Layout.tsx
export const {Name}Layout = createModuleLayout({
  moduleName: "{name}",
  getModuleConfig: (t) => ({
    title: t("{name}.module.title"),
    subtitle: t("{name}.module.subtitle"),
    icon: "Truck",
    hideBrand: true,  // ← 建议：Gateway App 视角下 Sidebar 不渲染品牌 block
    accentBarStyle: "solid",
    accentBarColor: "#1a6e97",
    // ...
  }),
  useNavItems,
});
```

**作用域说明**：

- Gateway 视角（`isAppPerspective === false`，如 `/` 首页）：模块 Layout 不参与渲染，Gateway TopBar 展示平台品牌名。
- Meta 预览 / 模块独立运行（`embedded: true` 但非 Gateway App 视角）：模块 `ModuleLayout` 全屏渲染包含 Sidebar + 内容区，不渲染品牌 block。
- **`shop` 模块**：使用双区域自定义布局（§11.9），不走 `createModuleLayout`，本节不适用。

#### 11.11.7 已使用 ModuleLayout 的模块

§11.8 全量模块对照表中标记为 "对齐模板" = ✅ 的模块均已使用 `createModuleLayout` 工厂。

#### 11.11.8 规范

- 模块布局必须使用 `createModuleLayout` 工厂（门禁：`scripts/check/check-module-auth-middleware.ts`）
- 认证由 Gateway 统一处理，模块 Layout 不定义 auth 中间件（门禁：`scripts/check/check-module-auth-middleware.ts`）
- 模块导航使用 Sidebar，不得在 Layout 中使用 `react-router` 导航守卫
- `shop` 模块使用独立双区域布局，不得使用 `ModuleLayout`
- Gateway App 视角下 Sidebar 顶部品牌 block 已整体移除，不得渲染独立品牌 block
  模块级 `TopBar` 由 `ModuleLayout` 内置渲染，是 Sidebar 与 ContentArea 之间的水平顶栏。本节定义其视觉样式、区域划分和交互行为，所有模块应在此规约约束下保持一致性。

##### 11.11.9.1 布局与尺寸

| 属性       | 规约值                            | 说明                                                                         |
| ---------- | --------------------------------- | ---------------------------------------------------------------------------- |
| 容器       | `<header>` flex 行布局            | 第一个子元素为 ContentArea                                                   |
| 高度       | **56px**                          | `h-14`，非 `h-16`（64px）                                                    |
| 水平内边距 | **24px** 或 `px-6`                |                                                                              |
| 子项间距   | **12px**                          | `gap-3`                                                                      |
| 定位       | 普通文档流                        | 非 sticky/fixed，flex 约束                                                   |
| 背景       | **纯色** `hsl(var(--background))` | 毛玻璃（`backdrop-blur-*`）由门禁拒绝（门禁：`scripts/check/check-frontend-layer-rules.ts`）；低透明度背景在模块内容区会导致层次混乱 |
| 底部边框   | `border-b`                        | 1px solid border                                                             |

##### 11.11.9.2 区域划分

```
┌──────────────────────────────────────────────────────────────┐
│ [tabs]              [searchSlot]              [actions]      │
└──────────────────────────────────────────────────────────────┘
```

- `tabs`（左侧）：面包屑或模块级 Tab
- `searchSlot`（中部）：模块级搜索
- `actions`（右侧）：Workspace 触发器（EmpAgent / 审批 / 收件箱 / 日程）
- 三段使用 `flex-1` / `flex-shrink-0` 约束，搜索区弹性扩展

##### 11.11.9.3 搜索框（searchSlot）

`searchSlot` 默认实现 `DefaultSearchSlot`（`ModuleLayout` 内置），规格：

- 宽度 288px（`w-72`），居中布局
- 左侧 `Search` 图标（lucide-react）
- placeholder 由 `moduleName.topBarSearchPlaceholderKey` 控制（i18n）

##### 11.11.9.4 Workspace Triggers（actions 插槽）

`actions` 插槽由 `useWorkspaceSlots({ ai, approval, schedule, inbox, profile })` 统一构建：

```ts
// Framework/frontend/components/src/components/layout/ModuleLayout.tsx
const { slots, pendingCount, unreadCount } = useWorkspaceSlots({
  ai: workspaceConfig?.ai,
  approval: workspaceConfig?.approval,
  schedule: workspaceConfig?.schedule,
  inbox: workspaceConfig?.inbox,
});
```

- 每个 trigger 是一个 `WorkspaceTrigger` 按钮（含 `Bell` / `Bot` / `ClipboardCheck` / `Calendar` 图标）
- Badge 角标自动从 `unreadCount` / `pendingCount` 读取
- 触发后打开对应 WorkspaceDock 面板（与 Sidebar 并列右侧）

##### 11.11.9.5 Accent Bar

模块内容区顶部 3px 强调色条，由 `moduleName.accentBarColor` + `moduleName.accentBarStyle` 控制：

```tsx
<div
  className={cn(
    'h-[3px] w-full',
    moduleName.accentBarStyle === 'solid' && !moduleName.accentBarColor && 'bg-primary',
    moduleName.accentBarStyle !== 'solid' && !moduleName.accentBarColor && 'bg-primary/15',
  )}
  style={moduleName.accentBarColor ? { backgroundColor: moduleName.accentBarColor } : undefined}
/>
```

此装饰条**当前未在 ModuleLayout 中实现**。选装约束：

- ✅ 仅 `accentBarColor` 控制颜色
- 色条样式由 `accentBarColor` 统一控制，必须从 `moduleAccent` CSS 变量继承

##### 11.11.9.6 与 Gateway TopBar 的关系

| 组件     | `ModuleLayout` 内置 `TopBar`       | `GatewayTopBar`（`FrameworkTopBar` 的组合）    |
| -------- | ---------------------------------- | ---------------------------------------------- |
| 渲染时机 | 模块独立运行时 / `embedded: false` | Gateway 视角 + App 视角                        |
| 高度     | 56px (`h-14`)                      | 56px (`h-14`)                                  |
| Logo     | 模块 icon + 名称                   | Gateway 品牌 logo + 应用名（App 视角下）       |
| Tabs     | 面包屑                             | 面包屑（Gateway 视角）/ ModuleTabs（App 视角） |
| Search   | 模块搜索                           | 跨模块全局搜索                                 |
| User     | 无（由 Gateway 接管）              | 头像 + 通知 + 设置 + 退出                      |

##### 11.11.9.7 规范

- 不得在模块页面内自定义 TopBar 替换 `ModuleLayout` 内置 TopBar
- TopBar 高度统一为 56px（`h-14`），不得使用 64px（`h-16`）
- TopBar 使用纯色背景，毛玻璃效果（`backdrop-blur-*`）由门禁拒绝（门禁：`scripts/check/check-frontend-layer-rules.ts`）
- embedded（Gateway 集成）模式下不得渲染模块级 TopBar

### 12.1 概述

树形表是展示层级数据的通用 UI 组件，适用于产品 BOM 展开、组织架构树、分类层级浏览、目录树等场景。
本规约定义树形表的布局、缩进、展开/折叠、数据汇总等共性技术模式，**不绑定具体业务语义**。

### 12.2 布局：CSS Grid

树形表**必须使用 CSS Grid 布局**，禁止使用 `<table>` 或 flexbox 模拟表格，以确保列宽与表头严格对齐：

```css
.tree-table-header,
.tree-table-row {
  display: grid;
  grid-template-columns: <业务定义各列宽度>;
  gap: 8px;
  padding: 7px 16px;
  align-items: center;
  white-space: nowrap;
}
```

表头和数据行的 `grid-template-columns` **必须保持一致**。列宽在定义时确定，数据行不使用 `flex-shrink` 或 `paddingLeft` 动态伸缩。

### 12.3 缩进实现

**禁止**使用行级 `paddingLeft` 做层级缩进（会导致整行列错位）。

正确做法：将缩进指示器（展开按钮 ▶/▾ + 层级连接线）与编码/名称文字**共同包裹在一个 flex 容器**内，作为 Grid 第一个单元格的内容：

```jsx
<div className="tree-table-row">
  <div style={{ display: 'flex', alignItems: 'center', gap: 2, overflow: 'hidden' }}>
    <span className="tree-table-toggle">{hasChildren ? (expanded ? '▾' : '▸') : ''}</span>
    {Array.from({ length: depth }).map((_, i) => (
      <span key={i} className="tree-table-level" />
    ))}
    <span className="tree-table-label">{node.label}</span>
  </div>
  {/* ... 其余 grid 子项按业务列顺序 */}
</div>
```

关键约束：

- 缩进指示器 + 层级线 + 标签文字**三者始终在同一行**（`white-space: nowrap`）
- 展开按钮（`▶`/`▾`）只有有子节点的行才显示，叶子节点保持占位（`visibility: hidden`）
- 层级连接线宽度固定（每级 `14px`），不影响后续列的位置

### 12.4 数据模型

树形表的节点数据模型应满足递归结构：

```typescript
interface TreeNode {
  id: string;
  label: string; // 首列显示文本
  children?: TreeNode[]; // 子节点，缺失或空数组视为叶子
  // ... 业务列字段（按需扩展）
}
```

### 12.5 展开/折叠

- 树形表初始化时**默认全部展开**（便于用户看到完整结构）
- 提供「全部展开/全部折叠」一键切换按钮
- 展开/折叠仅切换指定节点，不级联到子节点
- 展开状态使用 `useState<Record<string, boolean>>` 管理，key 为节点 `id`

```typescript
const [expanded, setExpanded] = useState<Record<string, boolean>>(() => {
  const all: Record<string, boolean> = {};
  function walk(nodes: TreeNode[]) {
    for (const n of nodes) {
      if (n.children && n.children.length > 0) {
        all[n.id] = true;
        walk(n.children);
      }
    }
  }
  walk(data);
  return all;
});
```

### 12.6 汇总栏（可选）

树形表顶部可显示统计栏，汇总当前展开范围的数据。指标由业务方定义，常见维度：

| 指标         | 计算方式                   |
| ------------ | -------------------------- |
| 节点总数     | 递归统计全部节点           |
| 唯一节点数   | Set 去重（按 id）          |
| 叶子节点数   | 无子节点的节点数           |
| 直接子节点数 | 根节点的 `children.length` |

### 12.7 节点交互（可选）

- **行点击**：选中/反选，联动详情面板或高亮
- **标签点击**：弹出详情模态/抽屉/跳转子页面
- **行悬停**：仅可操作元素（展开按钮、标签文字）呼应，纯展示元素不 hover

### 12.8 列头对齐

表头列若对应数据行带 padding 的组件（如状态 badge 有 `padding: 2px 8px`，类型 badge 有 `padding: 1px 6px`），表头对应列**必须使用相同的 padding**，避免视觉错位：

```jsx
<span style={{ padding:'1px 6px' }}>类型</span>
<span style={{ padding:'2px 8px' }}>状态</span>
```

### 12.9 树形表构建规范

- 树形表必须使用 CSS Grid，不得使用 `<table>` 或 flexbox
- 层级缩进使用 CSS padding/margin，不得使用动态 `paddingLeft` inline style
- 表头列 padding 必须与数据行组件 padding 一致
- 树形表行必须 `white-space: nowrap`，禁止行内换行
- 树形表不得使用入场动画或行过渡动画
- 展开按钮占位必须使用 `visibility: hidden`，不得使用 `display: none`

### 13.1 弹窗组件规范

前端代码 MUST 使用项目内置的 `AlertDialog` / `Dialog` 组件（原生浏览器弹窗 API 由门禁拦截；门禁：`scripts/check/check-no-native-dialog.ts`）：

| 原生 API           | 替代组件           | 来自                 |
| ------------------ | ------------------ | -------------------- |
| `window.alert()`   | `AlertDialog`      | `@alioth/components` |
| `window.confirm()` | `AlertDialog`      | `@alioth/components` |
| `window.prompt()`  | `Dialog` + `Input` | `@alioth/components` |

**原因**：

- 原生对话框样式与项目设计系统不一致（白底/蓝边/i18n 不可定制）
- 阻塞 JS 事件循环，影响 React 状态管理与 SSE 流式任务
- 不可定制位置、大小、动画，违反 `HTML_DESIGN_SPEC §12` 模态框规约
- 浏览器差异巨大，无法支持项目多主题切换

### 13.2 危险操作必须用 `destructive` 变体

删除、清除、强制重置、终止任务等**不可撤销**操作，确认按钮**必须**使用 shadcn `destructive` 变体：

```tsx
// ✅ 正确 — 用 destructive 设计 token，自动跟随主题
<AlertDialogAction
  onClick={handleClear}
  className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
>
  确定清除
</AlertDialogAction>
```

`destructive` 变体已集成在 `Framework/frontend/components/src/components/ui/button.tsx` 的 `buttonVariants`，无需自定义：

```typescript
const buttonVariants = cva(..., {
  variants: {
    variant: {
      default: "bg-primary text-primary-foreground hover:bg-primary/90",
      destructive: "bg-destructive text-destructive-foreground hover:bg-destructive/90",
      // ...
    }
  }
});
```

### 13.3 文本颜色必须用设计 tokens

弹窗标题、描述文本颜色**必须**使用 Tailwind 主题色 token（raw 颜色由门禁拦截；门禁：`scripts/check/audit-css-framework.mjs`）：

| 元素                          | 推荐 token              | 用途                   |
| ----------------------------- | ----------------------- | ---------------------- |
| 标题 `AlertDialogTitle`       | `text-foreground`       | 标题正文（默认已继承） |
| 描述 `AlertDialogDescription` | `text-muted-foreground` | 描述/说明（已内置）    |
| 错误提示                      | `text-destructive`      | 错误/失败状态          |
| 成功提示                      | `text-success`          | 成功状态               |

```tsx
// ✅ 正确
<AlertDialogTitle className="text-foreground">操作确认</AlertDialogTitle>
<AlertDialogDescription>此操作不可撤销</AlertDialogDescription>

```

### 13.4 弹窗结构标准骨架

确认对话框**必须**遵循以下结构，引用项目规约 `HTML_DESIGN_SPEC §12`：

```tsx
<AlertDialog open={open} onOpenChange={setOpen}>
  <AlertDialogContent>
    <AlertDialogHeader>
      <AlertDialogTitle className="text-foreground">操作标题</AlertDialogTitle>
      <AlertDialogDescription>操作说明，含影响范围与是否可撤销</AlertDialogDescription>
    </AlertDialogHeader>
    <AlertDialogFooter>
      <AlertDialogCancel>取消</AlertDialogCancel>
      <AlertDialogAction
        onClick={handleConfirm}
        className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
      >
        确定操作
      </AlertDialogAction>
    </AlertDialogFooter>
  </AlertDialogContent>
</AlertDialog>
```

| 元素     | 要求                                                         |
| -------- | ------------------------------------------------------------ |
| 标题     | 必须 `text-foreground`，不得裸色                             |
| 描述     | 必须含「不可撤销」/「影响范围」等风险提示                    |
| 取消按钮 | `AlertDialogCancel`（自动 `outline` 变体）                   |
| 确认按钮 | 危险操作必须 `destructive` 变体；普通操作默认 `default` 变体 |

### 13.5 错误提示弹窗

后端 API 调用失败、用户输入校验失败等场景，使用专用错误弹窗（`alert()` 由门禁拦截；门禁：`scripts/check/check-no-native-dialog.ts`）：

```tsx
function SomeAction() {
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const handleAction = async () => {
    try {
      await api.doSomething();
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : '操作失败');
    }
  };

  return (
    <>
      <button onClick={handleAction}>执行</button>
      <AlertDialog open={!!errorMessage} onOpenChange={(o) => !o && setErrorMessage(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle className="text-foreground">操作失败</AlertDialogTitle>
            <AlertDialogDescription>{errorMessage ?? ''}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogAction onClick={() => setErrorMessage(null)}>确定</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}
```

---

## 14. Tailwind v4 与 CSS 源扫描规约

### 14.1 `@source` 路径必须指向 monorepo 根

> **2026-06-12 修复**：项目从 Tailwind v3 迁移到 v4 后，`@source` 路径配置错误的 CSS 类未生成，曾导致所有 `AlertDialog` / `Dialog` / `Sheet` 的响应式类（`sm:flex-row`、`sm:justify-end`、`sm:space-x-2`）缺失，UI 严重错位。

`Framework/frontend/components/src/**` 中的组件使用的 Tailwind 类**不会**被 Vite 默认源扫描到（不在应用入口的依赖图上），必须通过 `@source` 显式声明。

**正确配置**（以 `Meta/frontend/src/styles/main.css` 为例）：

```css
@import 'tailwindcss';

/* ⚠️ 路径必须从 main.css 到 monorepo 根的 ../../../../  */
@source "../../../../Framework/frontend/components/src";
```

**路径深度计算**：`Meta/frontend/src/styles/main.css` → `../`(src/) → `../../`(frontend/) → `../../../`(Meta/) → `../../../../`(monorepo 根) → `../../../../Framework/frontend/components/src`。

| 应用      | main.css 位置                                 | 正确 `@source` 路径                                  |
| --------- | --------------------------------------------- | ---------------------------------------------------- |
| `Meta`    | `Meta/frontend/src/styles/main.css`           | `"../../../../Framework/frontend/components/src"`    |
| `Gateway` | `Gateway/frontend/src/index.css`              | `"../../../Framework/frontend/components/src"`       |
| `SSO`     | `SSO/frontend/src/styles/main.css`            | `"../../../../Framework/frontend/components/src"`    |
| 各 Module | `Pre-Proc/{ns}/Sources/{Apps｜Open｜Services｜changes}/Modules/<name>/frontend/src/theme.css`（或 `index.css` / `dev-styles.css`） | 声明 `@source` 时从该 CSS 文件到根为 7 层：`"../../../../../../../Framework/frontend/components/src"` |

**模块源码必须由宿主应用声明**（2026-09-14 修复）：模块页面的 Tailwind 工具类由 Gateway 的
`index.css` 统一生成，因此 Gateway MUST 声明模块源码 glob，且 glob 中的 `Sources` 与
`Modules` 之间有一层容器目录（`Apps` / `Open` / `Services` / `changes`）：

```css
/* 模块源码（Sources 下一层容器）——漏写该层会零命中，模块工具类静默不生成 */
@source "../../../Pre-Proc/*/Sources/*/Modules/*/frontend/src/**/*.ts";
@source "../../../Pre-Proc/*/Sources/*/Modules/*/frontend/src/**/*.tsx";
```

> 实证：写成 `Pre-Proc/**/Sources/Modules/**` 时 `@tailwindcss/oxide` Scanner 命中 **0 文件**
> （修后 609 文件 / 12690 候选类），模块独有的 `w-3/5` / `w-2/5` / `md:grid-cols-5` 等类
> 全部未生成，页面退回内容宽度（合同详情页右列仅 370px，行尾留 355px 空白）。

**判据要求**：

- 路径深度必须正确（如 Meta 的 `"../../Framework/frontend/components/src"` 需校对）
- 路径必须指向存在的目录（不指向已删除或重命名的目录；门禁：`scripts/check/check-tailwind-source.sh`）
- 引用共享组件时必须使用 `@source` 指令（依赖 Vite 自动扫描 → 共享组件的 Tailwind 类丢失）

### 14.2 全量审计（CI 强制检查）

> **2026-06-12 升级**：从抽检 `sm:flex-row` 三个类升级为**全量**检查所有带响应式/状态前缀的 Tailwind 类（184+ 个），覆盖 `Framework/frontend/components/src/**` 中每个使用的 `sm:` / `md:` / `dark:` / `hover:` / `data-[]` 等前缀类。

项目提供专用审计脚本 **`scripts/check/check-tailwind-source.sh`**，集成到 `test-all.sh --check` 和 `.gitea/workflows/meta_cicd.yaml`：

```bash
# 完整检查（构建所有 app + 验证 @source 路径 + 验证所有前缀类覆盖度）
bash scripts/check-tailwind-source.sh

# 仅检查（要求 dist 已存在）
bash scripts/check-tailwind-source.sh --no-build

# 仅检查单个 app
bash scripts/check-tailwind-source.sh Meta

# 严格模式（任何 warning 也算失败）
bash scripts/check-tailwind-source.sh --strict
```

审计脚本三层验证：

1. **路径深度**：应用/模块 `@source` 相对路径深度（Meta=4, Gateway=3, SSO=4；模块 CSS 入口=7）
2. **路径可解析**：`@source` 指向的目录必须在磁盘上存在
3. **CSS 覆盖度**：扫描 `@source` 目录下所有 `.tsx`/`.ts`/`.jsx`/`.js` 文件，提取每个带前缀的 Tailwind 类，逐类验证 dist CSS 中是否生成了对应规则（类名中非 `[A-Za-z0-9_-]` 的字符按 Tailwind 转义为 `\<char>` 形式匹配，如 `w-3/5` → `.w-3\/5`、`md:grid-cols-5` → `.md\:grid-cols-5`）
   - Gateway 的覆盖范围 MUST 与其 `index.css` 的 `@source` 声明对齐：`Framework/frontend/components/src`、`Framework/frontend/composables/src`、`Pre-Proc/*/Sources/*/Modules/*/frontend/src`、`Gateway/frontend/src`
   - 模块源码目录发现数为 0 时脚本 MUST 失败（glob 漂移哨兵），不得静默跳过
   - ⚠️ 2026-09-14 修复前该层判定恒真（`missing` 数组从未填充、`css_concat` 计算后未使用），无论缺多少类都输出 PASS——修复后必须实测「缺失类导致退出码 1」才能确认门禁生效

脚本退出码：

- `0`：全部通过
- `1`：存在 @source 路径错误或前缀类缺失

**预期输出示例**：

```
▶ 0. 验证各应用 main.css @source 路径
  [PASS] Meta: @source 路径正确 → ../../../../Framework/frontend/components/src
▶ 3. 验证 dist CSS 中所有前缀类
  [PASS] Meta: 全部 184 个前缀类已生成到 dist CSS
✅ 全部 @source 路径正确,无前缀类缺失
```

### 14.3 共享组件颜色 tokens 优先级

Framework 共享组件（`AlertDialog` / `Dialog` / `Sheet` / `Popover` 等）**必须**使用以下 Tailwind 主题色 token，**禁止** raw 颜色：

| Token                                            | 含义     | 替代 raw color                       |
| ------------------------------------------------ | -------- | ------------------------------------ |
| `bg-background`                                  | 背景     | `#fff`、`white`                      |
| `text-foreground`                                | 前景文字 | `#000`、`black`、`#1a1a1a`           |
| `text-muted-foreground`                          | 弱化文字 | `text-gray-400`、`text-gray-500`     |
| `bg-primary` / `text-primary-foreground`         | 主色     | 项目主色 raw 值                      |
| `bg-destructive` / `text-destructive-foreground` | 危险     | `bg-red-500`、`text-red-400`         |
| `bg-accent` / `text-accent-foreground`           | 强调     | hover/active 状态                    |
| `border-border` / `border-input`                 | 边框     | `border-gray-200`、`border-gray-300` |

这些 token 在 `Framework/frontend/components/src/theme-base.css` 与各应用 `main.css` 的 `@theme` 块中定义，遵循 shadcn/ui 主题系统。

### 14.4 避免 `mt-N sm:mt-0` 组合（2026-06-12 教训）

> **2026-06-12 案例**：`AlertDialogCancel` 默认带 `mt-2 sm:mt-0`，但因 Tailwind v4 CSS 源顺序中 `.mt-2` (位置 1537) 在 `.sm:mt-0` (位置 1285) **之后**，同特异性下 `mt-2` 覆盖 `sm:mt-0`，导致取消按钮永远下移 8px，与确认按钮错位。

**禁止**在共享组件中使用 `mt-N sm:mt-0` 之类的「响应式重置 margin」组合：

- 避免 `mt-2 sm:mt-0` — 源顺序下永远 8px top margin
- 避免 `mr-2 sm:mr-0`
- 避免任何 `margin-{side}-N sm:margin-{side}-0` 组合

**正确做法**：间距由 flex 容器统一管理（`gap-2` / `gap-4`），不依赖子元素 margin 重置。

```tsx
// ✅ 正确：AlertDialogFooter 用 gap-2 控制子项间距
<div className="flex flex-col-reverse gap-2 sm:flex-row sm:justify-end sm:space-x-2">
  <AlertDialogCancel>取消</AlertDialogCancel>   {/* 不需要 mt-2 */}
  <AlertDialogAction>确定</AlertDialogAction>
</div>

<div className="flex flex-col-reverse sm:flex-row sm:justify-end sm:space-x-2">
  <AlertDialogCancel className="mt-2 sm:mt-0">取消</AlertDialogCancel>   {/* 源顺序下不生效 */}
  <AlertDialogAction>确定</AlertDialogAction>
</div>
```

**规约**：Framework 共享组件中**禁止**出现 `mt-N sm:mt-0` / `mr-N sm:mr-0` / `mb-N sm:mb-0` / `gl-N sm:gl-0` 组合，必须用 `gap` 由父容器控制间距。

## 15. SSE 事件与前端步骤对齐规约

### 15.1 前端预定义步骤必须与后端 ProgressEvent 严格对齐（2026-06-12 教训）

> **2026-06-12 案例**：`ModelPublishPage.tsx` 的 `PUBLISH_STEPS` 包含 8 个步骤（`prepare`, `schema`, `enums`, `isahl_dump`, `meta_dump`, `meta_data`, `readme`, `done`），但后端 `Meta/backend/src/model_publish.rs` 的 `publish_model_to_dir` 只发送 5 个 ProgressEvent（`prepare`, `schema`, `isahl_dump`, `readme`, `done`）。前端 3 个步骤永远停在"等待中..."，即使发布已成功完成。

**核心规约**：使用 SSE 流式进度的前端，**预定义的步骤列表必须 1:1 对应后端会发送的 ProgressEvent 名称**。禁止列出后端不发事件的步骤，也禁止等待后端不发的事件。

**对齐清单**（必须保持一致）：

| 步骤 ID（step） | 显示标签          | 后端发送者                               |
| --------------- | ----------------- | ---------------------------------------- |
| `prepare`       | 准备输出目录      | `model_publish.rs::publish_model_to_dir` |
| `schema`        | 创建设置 SQL      | 同上                                     |
| `isahl_dump`    | 导出 isahl 表结构 | 同上                                     |
| `readme`        | 写入模型文件      | 同上                                     |
| `done`          | 发布完成          | 同上                                     |

**禁止**：

- 步骤定义必须前后端对齐：前端不得定义后端不发的步骤（如 `enums`/`meta_dump`/`meta_data`）
- 后端事件名变更时必须同步更新前端（后端改名 `isahl_dump` → `dump_isahl` 后必须同步修改）

**修改流程**：后端新增/删除 ProgressEvent 时，**必须同时修改**：

1. 后端 `*_model_publish.rs`（或对应后端文件）
2. 前端 `PUBLISH_STEPS` 常量
3. 文档 `docs/specs/MODULE_SPEC.md §15.1` 对齐清单

### 15.2 时间线卡片必须可截断（overflow 防护）

> **2026-06-12 案例**：`VersionTimeline` 在 `lg:grid-cols-[2fr_1fr]` 布局下，1fr 栏（时间线栏）实际可用宽度仅 ~250px，但卡片内 inline-flex 元素（版本号 70px + 状态徽章 60px + "验证"按钮 40px + padding 24px + gap 16px = 210px+）在 `failed` 状态下"验证失败"（4 字符）比 `verified` "已验证"（3 字符）宽 10-15px，触发**整体超出卡片背景**。用户报告"验证"按钮悬空在卡片外。

> **修复**：
>
> 1. 父容器加 `flex-1 min-w-0` 让 inline-flex 受约束
> 2. inline-flex 加 `max-w-full overflow-hidden`
> 3. 文本节点加 `truncate min-w-0`，让长内容截断而非溢出
> 4. 按钮 padding `px-1.5 py-0.5` → `px-1 py-0.5`
> 5. 状态徽章 `px-1.5 py-0.5 gap-1` → `px-1 py-0.5 gap-0.5`

**所有时间线/列表/侧栏卡片内嵌横向元素时，必须遵循以下规则**：

| 规则                         | 实现                                                             |
| ---------------------------- | ---------------------------------------------------------------- |
| 父容器必须可收缩             | `flex-1 min-w-0`                                                 |
| 容器不允许溢出               | `max-w-full overflow-hidden`                                     |
| 文本元素必须可截断           | `truncate min-w-0`                                               |
| 关键操作（按钮）必须不被截断 | `shrink-0`                                                       |
| Padding/Gap 必须紧凑         | 侧栏内嵌元素用 `gap-1.5` `px-2 py-1.5`，不用 `gap-2` `px-3 py-2` |

### 15.3 历史失败状态自动重试

> **2026-06-12 案例**：数据库中存在 `v10.0.100` 等 `status = failed` 的历史失败记录。在发布流程修复后，这些记录**已经可以重试通过**，但停留在 `failed` 状态，前端需要用户逐个点击"验证"按钮才能清理。8 秒自动重置只在用户主动点击时触发。

> **修复**：在 `VersionTimeline` 挂载时，对所有 `status === "failed"` 的记录**自动调用 `handleVerify` 重试一次**。失败仍然按原逻辑处理（8 秒后自动重置回 `published`）。

```tsx
React.useEffect(() => {
  if (!versions) return;
  const failedVersions = versions.filter((v) => v.status === 'failed');
  if (failedVersions.length === 0) return;
  failedVersions.forEach((v) => handleVerify(v.version));
}, [versions?.length]);
```

**禁止**：

- 历史 `failed` 状态不得永久残留时间线
- 不得强迫用户手动点击每个失败记录的"验证"按钮清理

## 16. 临时状态与错误提示规约

> **核心问题（2026-06-12 案例）**：`useState<Record<Id, T>>` 是"持续累积无清理"的反模式，根源是它没有与"数据源生命周期"绑定。常见问题：
>
> 1. 数据源已删除 item, ephemeral 状态残留在 Map 中
> 2. 异步操作完成后, 客户端状态与 query 数据时序错位
> 3. 错误对话框 `setErrorMessage + AlertDialog` 永远不会被关闭, 错误消息永久占用 UI
> 4. 多个 setTimeout / dismissTimers ref 散落, 难以追踪, 容易泄漏

### 16.1 ephemeral 状态必须绑定到数据源生命周期

**核心原则**：所有「与某条数据项关联的临时状态」（如"已验证"、"失败"、临时编辑值、临时选中状态），**必须**使用 `useEphemeralState` Hook，并显式声明与数据源的生命周期同步。

**禁止**：

- `const [verifyResults, setVerifyResults] = useState<Record<Id, T>>({})` — 必须设置自动清理
- 多个项的状态必须使用统一的 dismiss 管理，不得用散落的 `setTimeout` / `dismissTimers` ref
- 派生 UI 状态（"已验证"徽章）不得与 query 数据中已有的状态字段重复

**正确做法**：

```tsx
import { useEphemeralState, useEphemeralStateSync } from '@alioth/components';

// 1. 声明 ephemeral state 容器（按 key 索引, 类似 Map）
const verifyResults = useEphemeralState<{ status: string; output: string | null }>();

// 2. 显式声明绑定到数据源生命周期
useEphemeralStateSync(
  verifyResults,
  versions?.map((v) => v.version) ?? [], // 数据源当前所有 key
  {
    strategy: 'remove-mismatched', // 策略: 清理不存在 OR 状态已同步的 entry
    getStatus: (key) => versions?.find((v) => v.version === key)?.status,
  },
);

// 3. 设置/读取/清除 API
verifyResults.setState('v10.0.100', { status: 'failed', output: '...' });
const r = verifyResults.getState('v10.0.100'); // EphemeralValue<T> | undefined
verifyResults.clearState('v10.0.100');
verifyResults.clearAll();

// 4. 渲染时判断 stale
const r = verifyResults.getState(v.version);
if (r && r.data.status !== v.status) {
  // 渲染"验证中..."徽章
}
// 当 DB 状态同步为 verified 后, useEphemeralStateSync 自动清理该 entry
```

**API 参考**：

| 方法                          | 说明                                                        |
| ----------------------------- | ----------------------------------------------------------- |
| `getState(key)`               | 获取 ephemeral value, 含 `data` + `createdAt` + `expiresAt` |
| `setState(key, data, ttlMs?)` | 设置, 可选 TTL 过期（暂未自动化过期, 留作未来扩展）         |
| `clearState(key)`             | 清除单个                                                    |
| `clearAll()`                  | 清除所有                                                    |

**同步策略**：

- `remove-stale`: 仅清理 data source 中已不存在的 key
- `remove-mismatched`: 清理不存在 **OR** ephemeral 状态已与 data source 一致

### 16.2 短暂错误用 Toast, 不用 setErrorMessage + AlertDialog

> **2026-06-12 案例**：`setErrorMessage + AlertDialog open={!!errorMessage}` 模式, 错误消息依赖用户主动点击"确定"关闭, 但若用户刷新页面或导航走, errorMessage 仍可能在某次 re-render 时被错误地保留, 弹出"幽灵对话框"。同时, 网络错误/服务端错误在用户没点击的情况下永久占用 UI。

**核心规约**：所有 **短暂**错误和成功反馈, **必须**使用 `useNotification` (sonner toast)：

- ✅ `notify.error("操作失败", { action: retryFn, actionLabel: "重试" })`
- ✅ `notify.success("已保存")`
- ✅ `notify.apiError(err)` — 自动从 Error 中提取友好消息
- ✅ `notify.promise(fetchData(), { loading, success, error })` — 自动三态切换

**门禁判定**（门禁：`scripts/check/check-no-native-dialog.ts`）：

- 错误反馈使用统一的 Toast/通知系统（`setErrorMessage` + `<AlertDialog open={!!errorMessage}>` 模式由门禁拦截）
- 多个 `AlertDialog` 堆叠由门禁拦截

**例外**（仍可使用 AlertDialog）：

- ✅ **不可逆的破坏性操作确认**（如"清除所有发布历史"、"删除组织"）
- ✅ **需要用户做选择的场景**（如"替换现有文件？"）

**判断标准**：如果操作是「已发生错误的反馈」或「已完成的成功提示」，**用 Toast**；如果是「即将发生危险操作的确认」或「需要决策」，**用 AlertDialog**。

### 16.3 多个 AlertDialog 不应同时存在

在同一组件树中, **禁止**同时挂载多个 AlertDialog（确认 + 错误 + 警告）。原因：

1. z-index 堆叠混乱
2. ESC 键只能关闭最上层
3. dialog 状态机难以追踪

**反模式**：

```tsx
<AlertDialog open={confirmClearOpen}>...</AlertDialog>  // 确认清除
<AlertDialog open={!!errorMessage}>...</AlertDialog>     // 错误提示
```

> **正确做法**：

```tsx
<AlertDialog open={confirmClearOpen}>...</AlertDialog> // 仅保留确认 dialog
// 错误用 useNotification().notify.apiError(err), 不用 AlertDialog
```

### 16.4 setTimeout 必须有 cleanup 路径

> - **2026-06-12 案例**：`handleVerify` 中有 `dismissTimers.current[version] = setTimeout(...)`, 定时器在组件 unmount 时**不会自动清理**, 8 秒后仍会执行 `setQueryData`, 在 Strict Mode 双调用 / 组件树切换时可能 throw。

**规则**：

1. 如果用 `setTimeout` 修改 React state, 必须在 `useEffect` 的 cleanup 中 `clearTimeout`
2. 或者用 `useEphemeralState` 让状态自动随数据源 prune, 就不需要 `setTimeout` cleanup
3. 永远不要在事件 handler 中创建 setTimeout 引用 ref 然后不清理

**反模式**：

```tsx
const dismissTimers = useRef<Record<string, TimeoutHandle>>({});
const handleClick = () => {
  dismissTimers.current[key] = setTimeout(() => { ... }, 8000);
  // 组件 unmount 时, dismissTimers.current[key] 不会被清理
};
```

**正确做法**（重构成 ephemeral state, 不需要 setTimeout）：

```tsx
// 用 useEphemeralStateSync 自动清理, handleVerify 只需要:
// 1. setQueryData (DB 状态)
// 2. notify 用户结果
// setTimeout 由 useEphemeralStateSync 的 stale 检测代替
```

### 16.5 不可逆操作必须有显式确认（强约束）

> **反向规约**：上述 16.2 说"成功/错误用 Toast", 但**不可逆的破坏性操作**（删除、清空、覆盖、强制重置）**必须**用 `<AlertDialog>` 让用户显式点击"确定"。原因：toast 自动消失, 用户无足够时间思考后果。

```tsx
// ✅ 正确: 不可逆操作用 AlertDialog
<AlertDialog open={confirmClearOpen} onOpenChange={setConfirmClearOpen}>
  <AlertDialogContent>
    <AlertDialogTitle>确认清除所有发布历史？</AlertDialogTitle>
    <AlertDialogDescription>此操作不可撤销...</AlertDialogDescription>
    <AlertDialogFooter>
      <AlertDialogCancel>取消</AlertDialogCancel>
      <AlertDialogAction
        onClick={handleClearHistory}
        className="bg-destructive text-destructive-foreground"
      >
        确定清除
      </AlertDialogAction>
    </AlertDialogFooter>
  </AlertDialogContent>
</AlertDialog>
```

### 16.6 Rules of Hooks 严格检查

> **2026-06-12 案例**：`VersionTimeline` 组件的 `useState` 声明曾被错误地分散在 async function 之后, 重复声明导致 React 报"Rendered more hooks than during the previous render"错误, 整个组件 state 损坏, "清除操作失败"。

**规则**：

1. **所有 hooks 必须在组件顶部一次性按顺序声明**, 不要在 function、early return 之后声明
2. 同一 hook 类型在组件内**只能声明一次**（`useState(false)` 不能出现 2 次）
3. 重复的 `const [x, setX] = useState(false);` 即使在不同行, 也是 bug
4. 在 `tsconfig.json` 启用 `react-hooks/exhaustive-deps` lint 规则

**Lint 检查**（`Framework/frontend/.eslintrc` 已配置）：

```json
{
  "rules": {
    "react-hooks/rules-of-hooks": "error",
    "react-hooks/exhaustive-deps": "warn"
  }
}
```
