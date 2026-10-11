//! provider 注册表**装载契约**集成测试（`outbound-provider-abstraction`）。
//!
//! 为什么必须是**独立测试二进制**：`install()` 基于进程级
//! `static REGISTRY: OnceLock<ProviderRegistry>` —— 同进程只能安装一次 ⇒
//! 「装载前行为」「二次安装被拒」这两条无法在共享单测进程内断言（会被先跑的用例污染）。
//! cargo 为每个 `tests/*.rs` 起独立二进制，正是该断言的正确落点。
//!
//! 又因为 `OnceLock` 的进程级一次性，本文件**只有一个 `#[test]`**（同一二进制内多函数
//! 的执行顺序无保证，拆分即引入顺序依赖的 flaky）。阶段在函数体内按序断言，失败信息带阶段语义。
//!
//! 规约：
//!   - `openspec/specs/outbound-provider-abstraction :: provider-registry-install-contract-test`
//!   - `openspec/specs/outbound-provider-abstraction :: provider-registry-fail-closed`
//!     （未注册代号 MUST fail-closed、错误文案含已注册集合 = 诊断面）
//!
//! 用法：`cargo test -p outbound-client`（随其他 target 一并编译运行）。

use std::sync::Arc;

use async_trait::async_trait;
use outbound_client::provider::{
    install, installed, registry, resolve, OutboundProvider, ProviderRegistry,
};

/// 最小 provider 实现：只回读派发键（认证面本测试不关心，用 trait 默认实现）。
struct Probe(&'static str);

#[async_trait]
impl OutboundProvider for Probe {
    fn provider_id(&self) -> &'static str {
        self.0
    }
}

fn probe(id: &'static str) -> Arc<dyn OutboundProvider> {
    Arc::new(Probe(id))
}

#[test]
fn registry_install_contract() {
    // ── 阶段 1：装载前 —— 未安装注册表必须可区分于「已装但为空」，且解析 fail-closed ──
    assert!(!installed(), "独立进程起始态 MUST 为未安装");
    assert!(
        registry().is_none(),
        "未安装 ⇒ registry() 返回 None（与「已装空表」不可混为一谈）"
    );
    let err = match resolve("fssc") {
        Ok(_) => panic!("未安装注册表时解析 MUST fail-closed"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("注册表未安装"),
        "错误文案须给出装载指引（组合根 MUST 启动期 install）: {err}"
    );

    // ── 阶段 2：装载 —— 两个代号，登记顺序与 ids() 的字典序解耦 ──
    let built = ProviderRegistry::new()
        .with(probe("fssc"))
        .and_then(|r| r.with(probe("wechat-pay")))
        .expect("两个不同代号 MUST 可登记");
    install(built).expect("首次 install MUST 成功");

    assert!(installed(), "install 后 installed() MUST 为真");
    let reg = registry().expect("install 后 registry() MUST 为 Some");
    let ids = reg.ids();
    assert_eq!(
        ids,
        vec!["fssc", "wechat-pay"],
        "ids() MUST 按字典序稳定（管理面目录按该序呈现）"
    );
    // 同进程内重复取值 MUST 稳定（只读面：注册表构造期填充，运行期不变）
    assert_eq!(ids, reg.ids(), "同进程内重复取 ids() MUST 一致");
    assert_eq!(reg.len(), 2);
    assert!(reg.contains("fssc"));

    // ── 阶段 3：解析命中 —— provider_id() 回读一致（派发键 = 实现自述键） ──
    let p = resolve("fssc").expect("已注册代号 MUST 可解析");
    assert_eq!(p.provider_id(), "fssc", "provider_id() MUST 与派发键一致");
    let q = resolve("wechat-pay").expect("第二个已注册代号 MUST 可解析");
    assert_eq!(q.provider_id(), "wechat-pay");

    // ── 阶段 4：未注册代号 fail-closed，且文案含已注册集合（诊断面） ──
    let err = match resolve("carrier") {
        Ok(_) => panic!("未注册代号 MUST fail-closed（不得回退默认 provider）"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(msg.contains("carrier"), "文案须含请求代号以便定位: {msg}");
    assert!(
        msg.contains("wechat-pay") && msg.contains("fssc"),
        "文案须含已注册集合（本 ns 构建实际装入的适配器）: {msg}"
    );

    // ── 阶段 5：二次安装被拒（OnceLock 一次性语义 ⇒ 两处装配即派发面不确定） ──
    let second = ProviderRegistry::new().with(probe("other")).expect("构造");
    let err = install(second).expect_err("二次 install MUST 报错，MUST NOT 静默覆盖");
    assert!(
        err.to_string().contains("只能调用一次"),
        "文案须指明一次性语义: {err}"
    );
    // 拒绝后既有注册表 MUST 未被破坏
    assert_eq!(
        registry().expect("仍为 Some").ids(),
        vec!["fssc", "wechat-pay"],
        "二次安装被拒后 MUST 保持原注册表不变"
    );
}
