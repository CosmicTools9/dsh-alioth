//! HTTP Handler — crud v2 泛型路由工厂
//!
//! 软删除 / NGAC 创建者关联 / RLS 头解析 / dk 回退链由 crud 框架泛型实现。

use actix_web::web;

pub fn register(cfg: &mut web::ServiceConfig) {
    use crate::models::todo::{CreateTodoRequest, Todo, UpdateTodoRequest};
    use crate::repositories::todo::TodoRepository;
    cfg.configure(crud::crud_routes::<
        Todo,
        CreateTodoRequest,
        UpdateTodoRequest,
        TodoRepository,
        common::error::AliothError,
    >("/service/todos"));
}
