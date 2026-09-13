// drift_guard（split 自 repository.rs 内嵌模块，④ 候选）
//
// 原 guard 校验 ontology_binding::coords_for_entity 对 RUNTIME_ENTITIES 全覆盖；
// 该 API 已随 ontology_binding 重构移除（现仅 resolve/resolve_conn，需 DB 连接异步调用）。
// guard 待 split-identity-org-repository change 对齐新 API 后重建（此处不保留失效断言）。
