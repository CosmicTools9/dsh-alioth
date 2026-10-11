#!/usr/bin/env bun
/**
 * openapi-doc.test.ts — 结构化路由抽取判据自证（change `fix-openapi-doc-coverage`）
 *
 * 覆盖在仓实际形态（实测自 5 ns 的 handler 源码）：
 *   · `web::scope` + 内层 `.route("", handler)` 标准 CRUD（Alioth `customer` 形态）
 *   · `web::resource("R").route(web::get().to(h)).route(web::post().to(h))` 多方法同路径合并
 *   · `cfg.route("P", web::get().to(h))` 直连形
 *   · `crud_routes::<…>("S")(cfg)` 语法糖展开（crud crate 的 scope + 5 法 + /batch）
 *   · 嵌套 scope（`web::scope("/a").service(web::scope("/b")…)`）
 *   · 挂载段**保留**（`/service/bin-locations` 形态，实测 CT：挂载段与 service.json id 不同名）
 *   · 不可判定形态不产出（宁缺勿滥）
 */

import { describe, expect, test } from "bun:test";
import { extractRouteEntries, normalizeRoutePath } from "./openapi-doc";

const entries = (src: string): Record<string, string[]> =>
  Object.fromEntries(
    extractRouteEntries(src)
      .map((e) => [normalizeRoutePath(e.path), [...e.methods].sort()] as const)
      .sort((a, b) => a[0].localeCompare(b[0])),
  );

describe("extractRouteEntries", () => {
  test("web::scope + 内层 route 标准 CRUD（Alioth customer 形态）", () => {
    const src = `
pub fn register(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/customers")
            .route("", web::get().to(list_customers))
            .route("", web::post().to(create_customer))
            .route("/{id}", web::get().to(get_customer))
            .route("/{id}", web::put().to(update_customer))
            .route("/{id}", web::delete().to(delete_customer)),
    );
}
`;
    expect(entries(src)).toEqual({
      "/customers": ["get", "post"],
      "/customers/{id}": ["delete", "get", "put"],
    });
  });

  test("web::resource 多方法绑定 + 方法链不产生重复条目前缀", () => {
    const src = `
cfg.service(
    web::resource("/waybills/{id}/loading")
        .route(web::post().to(submit_loading)),
);
cfg.service(web::resource("/kanban").route(web::get().to(get_kanban)));
`;
    expect(entries(src)).toEqual({
      "/kanban": ["get"],
      "/waybills/{id}/loading": ["post"],
    });
  });

  test('cfg.route("P", handler) 直连形', () => {
    const src = `cfg.route("/status", web::get().to(status));`;
    expect(entries(src)).toEqual({ "/status": ["get"] });
  });

  test('crud_routes::<…>("S") 展开（scope + 5 法 + /batch delete）', () => {
    const src = `
pub fn register(cfg: &mut web::ServiceConfig) {
    crud_routes::<
        UnitConversionRate,
        CreateUnitConversionRateRequest,
        UpdateUnitConversionRateRequest,
        UnitConversionRateRepository,
        AliothError,
    >("/conversion-rates")(cfg);
}
`;
    expect(entries(src)).toEqual({
      "/conversion-rates": ["get", "post"],
      "/conversion-rates/{id}": ["delete", "get", "put"],
      "/conversion-rates/batch": ["delete"],
    });
  });

  test("嵌套 scope 前缀合成", () => {
    const src = `
cfg.service(web::scope("/a").service(
    web::scope("/b").route("/c", web::get().to(h)),
));
`;
    expect(entries(src)).toEqual({ "/a/b/c": ["get"] });
  });

  test("挂载段保留（CT `/service/bin-locations` 形态）", () => {
    const src = `
cfg.configure(crud::crud_routes::<
    BinLocation, C, U, R, AliothError,
>("/service/bin-locations"));
`;
    expect(entries(src)).toEqual({
      "/service/bin-locations": ["get", "post"],
      "/service/bin-locations/{id}": ["delete", "get", "put"],
      "/service/bin-locations/batch": ["delete"],
    });
  });

  test("正则约束 id 模板归一（{id:\\d+} → {id}）且作用域根 route(handler) 落到前缀", () => {
    const src = `
cfg.service(
    web::scope("/a").route(web::get().to(list)),
);
cfg.service(web::resource("/x").route(web::delete().to(rm_id)));
`;
    expect(entries(src)).toEqual({
      "/a": ["get"],
      "/x": ["delete"],
    });
  });

  test("注释与字符串正文不产生条目（不可判定形态不产出）", () => {
    const src = `
// cfg.route("/ghost", web::get().to(h));
/* web::resource("/ghost-2").route(web::post().to(h)) */
let label = "web::resource(\\"/ghost-3\\")";
`;
    expect(entries(src)).toEqual({});
  });

  test("非 PEP 映射面动词（head/options）仍被抽取（生成侧决定不落契约）", () => {
    const src = `cfg.service(web::resource("/probe").route(web::head().to(h)));`;
    expect(entries(src)).toEqual({ "/probe": ["head"] });
  });
});
