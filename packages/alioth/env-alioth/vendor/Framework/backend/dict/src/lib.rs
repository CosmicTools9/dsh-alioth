//! dict — 动态字典表 CRUD 共享内核（split-wz-isahl-db）
//!
//! 自 WZ isahl-db 上移（原样搬移，生产验证）：覆盖字典族（zc_id_cate-*/zc_id_tags-* 66 张），
//! 动态表 CRUD 委托 `crud::SchemaRepository` + 表名白名单 + NGAC object_attribute 注册。
//! 挂载：壳 scope 内 `dict::register(cfg)`（路由相对路径 /dict/*）。

pub mod handler;

pub use handler::register;
