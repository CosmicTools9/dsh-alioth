# blockAssembly 字段参考

> module.json 顶级字段，声明 Module 如何组装 Blocks。由 alioth-design Track 1N（组装原型设计）写入。

## 完整 Schema

```jsonc
// module.json 新增顶级字段
{
  "mode": "multi-block",
  "shell": "ModuleLayout",
  "navigation": {
    "groups": [
      {
        "id": "execution",
        "label": "项目执行",
        "icon": "Calendar",
      },
    ],
    "defaultBlock": "project-list",
    "collapseBehavior": "width",
  },
  "stateContract": {
    "shared": ["globalQuery", "userContext"],
    "isolated": ["search", "filter", "page", "selectedId"],
  },
  "blocks": [
    {
      "id": "project-list",
      "label": "项目列表",
      "group": "execution",
      "order": 0,
      "icon": "ListTree",
    },
  ],
  "serviceBindings": {
    "project-list": { "services": ["orchestration"] },
    "gate-overview": { "services": ["monitor", "commitment"] },
  },
}
```

## 配套顶级字段

```jsonc
// module.json 配套顶级字段
"blocks": [
  {
    "id": "transport-execution",            // 必填，对应 blockAssembly.blocks[].id
    "group": "操作台"                       // 必填，所属导航分组（展示用）
  }
]
```

## 与现有字段的关系

| 字段               | 关系                                                                   |
| ------------------ | ---------------------------------------------------------------------- |
| `extensionPoints`  | 保留，Block 组装下扩展点语义变为"Block 级逻辑钩子"而非"实体 CRUD 钩子" |
| `dependencies`     | `serviceBindings` 是 API 级别依赖声明，比 dataContracts 更细粒度       |
| `prototypeVersion` | 继续使用，但指向**组装原型**的版本号                                   |

## 用词规范

- `primaryPost`（岗位）替代 `primaryRole`（角色）
- `workbenchPosts` 替代 `workbenchRoles`
- 岗位是流程中的位置，"角色"保留给 NGAC 权限域
- `defaultBlock` 是 `blockAssembly.navigation` 的首屏默认字段，替代旧 `defaultScene`

## 验证

```bash
bun .agents/skills/alioth-module/scripts/validate-block-assembly.ts Pre-Proc/{ns}/Sources/Modules/{name}/module.json
bun .agents/skills/alioth-module/scripts/audit-assembly-prototype.ts Pre-Proc/Alioth/Prototypes/Modules/{name}/{name}-assembly-v{N}.html --module Pre-Proc/{ns}/Sources/Modules/{name}
```
