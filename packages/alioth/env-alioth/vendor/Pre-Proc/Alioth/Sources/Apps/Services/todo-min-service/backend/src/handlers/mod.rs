//! HTTP Handler — crud v2 泛型路由工厂
//!
//! 软删除 / NGAC 创建者关联 / RLS 头解析 / dk 回退链由 crud 框架泛型实现。

use actix_web::web;

pub fn register(cfg: &mut web::ServiceConfig) {
    use crate::models::todo_min::{CreateTodoMinRequest, TodoMin, UpdateTodoMinRequest};
    use crate::repositories::todo_min::TodoMinRepository;
    cfg.configure(crud::crud_routes::<
        TodoMin,
        CreateTodoMinRequest,
        UpdateTodoMinRequest,
        TodoMinRepository,
        common::error::AliothError,
    >("/service/todo-mins"));
}
