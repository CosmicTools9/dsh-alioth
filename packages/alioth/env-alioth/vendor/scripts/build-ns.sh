#!/bin/bash
# build-ns.sh — 按 namespace 编译 standalone binary 并输出到 Deploy/{ns}/bin/
#
# 用法:
#   bash scripts/build-ns.sh WZ [profile] [--clean]
#   bash scripts/build-ns.sh Alioth [profile]
#   bash scripts/build-ns.sh AVIC-CAASEC [profile]
#
# profile 默认 release，可指定 dev
# --clean 在构建前移除该 ns 的隔离编译缓存 Deploy/{ns}/target（本次将全量重建）；
#         MUST NOT 触碰 Deploy/{ns}/bin/（运行中实例可能持有该二进制）
#
# Meta 不是 namespace，不接受本脚本的参数：其权威构建 = scripts/meta/build.sh
# （产物 Meta/backend/target/），打包 = scripts/meta/deploy.sh --target Release/Meta。
#
# Framework/backend/   ← 根 workspace：共享基础设施 + SSO + Gateway
# Pre-Proc/{ns}/       ← 独立 workspace，各自有独立的 Cargo.lock + target 目录
# 构建 gateway binary：build-ns.sh 在 Gateway/backend 下执行
#   cargo build --manifest-path Gateway/host/Cargo.toml -p gateway-host --no-default-features --features {ns},sso --target-dir Deploy/{ns}/target
# Service crates 跨 workspace 通过 path deps 引用，Cargo 自动解析。
# 日常开发：cd Pre-Proc/{ns} && cargo check -p {ns}-service-xxx

set -euo pipefail

# 参数解析：位置参数 <namespace> [profile] + 可选旗标 --clean（位置无关）
CLEAN=false
POSITIONAL=()
for _arg in "$@"; do
  case "$_arg" in
    --clean) CLEAN=true ;;
    *) POSITIONAL+=("$_arg") ;;
  esac
done

NS="${POSITIONAL[0]:?Usage: build-ns.sh <namespace> [profile] [--clean]}"
PROFILE="${POSITIONAL[1]:-release}"
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# 依赖源择源（多镜像回退）：项目级 .cargo/config.toml 缺失时择源一次——避免 cargo 卡在不可达
# 用户级源并长时间持有包缓存锁（多会话互相阻塞）。实现见 lib/cargo-run.sh。
if [ -f "${PROJECT_ROOT}/scripts/lib/cargo-run.sh" ]; then
    # shellcheck source=scripts/lib/cargo-run.sh
    # shellcheck disable=SC1091
    source "${PROJECT_ROOT}/scripts/lib/cargo-run.sh"
    cargo_ensure_source_config "${PROJECT_ROOT}"
fi

# 入参归一：namespace 名大小写不敏感（唯一实现 = scripts/lib/ns-name.sh）。
# 调用点（scripts/lib/env-common.sh、prod/pre 启动链、restart-gateway.sh）历史上传小写
# `${NS:-alioth}`；不归一时 `Deploy/alioth/target` 会与 `Deploy/Alioth/target` 分裂出两份缓存。
source "$PROJECT_ROOT/scripts/lib/ns-name.sh"
NS="$(ns_canon "$NS")"
NS_LOWER="$(ns_lower "$NS")"

# 二进制落盘唯一实现（同目录临时文件 + rename）：直接 cp 覆盖运行中实例持有的文件会
# ETXTBSY（`文本文件忙`）致整个构建失败，见该库头部。
source "$PROJECT_ROOT/scripts/lib/atomic-install.sh"

# 触发线检测（add-build-threshold-check）：二进制 >500MB → 失败（时间线已移除）
source "$PROJECT_ROOT/scripts/lib/build-thresholds.sh"

# 交付面体积收敛（reduce-build-artifact-size）：strip → provenance xattr → ad-hoc 重签
# 三步固定顺序、不可拆（strip 会使签名失效）；未 strip 的 cargo 输出保持原样供定位
source "$PROJECT_ROOT/scripts/lib/artifact-trim.sh"

if [ "$PROFILE" = "release" ]; then
  CARGO_FLAGS="--release"
  TARGET_DIR_SUFFIX="release"
  # release: 关闭 incremental（只缓存全量编译产物）
  export CARGO_PROFILE_RELEASE_INCREMENTAL=false
else
  CARGO_FLAGS=""
  TARGET_DIR_SUFFIX="debug"
fi

# ── sccache 编译缓存接线（唯一实现 = scripts/lib/sccache.sh）────────────────────
# 状态文案只能由该实现产生（MUST NOT 在本脚本或别处硬编码「已启用」）；未接线/已停用时
# 由其打印原因与启用方式。接线面覆盖 debug 与 release 两种 profile（跨 ns / 跨 target
# 目录复用依赖编译），门禁见 scripts/check/check-sccache-wiring.ts。
# shellcheck source=scripts/lib/sccache.sh
source "${PROJECT_ROOT}/scripts/lib/sccache.sh"
sccache_enable_if_available "build-ns ${NS} ${PROFILE}"

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Building standalone binary for namespace: $NS"
echo "  Profile: $PROFILE"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"


# ── namespace 白名单校验 ────────────────────────────────────────────────
# 先于清理与构建：未知值 MUST 在任何删除动作前被拒绝（--clean 的路径护栏）。
case "$NS" in
  Alioth|WZ|AVIC-CAASEC|SE|Cosmic-Tools) ;;
  *)
    echo "❌ Unknown namespace: $NS"
    echo "   Supported namespaces: Alioth, WZ, AVIC-CAASEC, SE, Cosmic-Tools"
    echo "   （Meta 不是 namespace：构建 = scripts/meta/build.sh；打包 = scripts/meta/deploy.sh --target Release/Meta）"
    exit 1
    ;;
esac

# ── 可选：清理隔离编译缓存（--clean）───────────────────────────────────────
# 清理目标 = 该 ns 的编译缓存 Deploy/{ns}/target（Gateway 与 OpenActivity 共用）。
# MUST NOT 触 bin/：运行中实例可能持有 Deploy/{ns}/bin/{ns}-server，删除会中断服务，
# 且无必要——产物由随后的构建替换。
# 路径由脚本内固定拼装（$NS 已经 ns_canon 归一 + 上方白名单校验），
# MUST NOT 取自外部输入或环境变量。
# 位置 MUST 在构建之前：本脚本的构建分支自带 cargo build，清理若后置会删掉刚产出的二进制。
if [ "$CLEAN" = true ]; then
  NS_TARGET_DIR="$PROJECT_ROOT/Deploy/$NS/target"
  # 变量一律用 ${VAR} 花括号边界：裸 $VAR 紧邻全角字符（如「（」）时，bash 在 set -u 下
  # 会把全角字符的首字节并入变量名 → `unbound variable`（本仓已两次踩到，见
  # fix(pre-push) 的同名缺陷）。MUST NOT 改写为裸 $-形态。
  if [ -d "${NS_TARGET_DIR}" ]; then
    echo "→ --clean: 移除编译缓存 ${NS_TARGET_DIR} ($(du -sh "${NS_TARGET_DIR}" 2>/dev/null | cut -f1))——本次将全量重建"
    rm -rf "${NS_TARGET_DIR}"
  else
    echo "→ --clean: 编译缓存不存在（${NS_TARGET_DIR}）——跳过清理"
  fi
  echo ""
fi

# ── SSO 形态（单一事实源 = Deploy/sso-mode.conf；读取器 = scripts/lib/ns-sso-mode.sh）──
# embedded → 内嵌 SSO（feature `sso`）；remote → 只反代（feature `sso-remote`）且独立 SSO
# 与 Gateway 同发布单元（Deploy/{ns}/bin/gateway-sso）。读取器对未登记 ns 非零退出（不做
# 隐默默认）。契约: openspec/specs/sso-remote-auth-proxy :: per-namespace-sso-mode-contract。
# shellcheck source=scripts/lib/ns-sso-mode.sh
source "$PROJECT_ROOT/scripts/lib/ns-sso-mode.sh"
SSO_FEATURE="$(ns_sso_feature "$NS")"

# ── 构建 ────────────────────────────────────────────────────────────────
case "$NS" in
  Alioth|WZ|AVIC-CAASEC|SE|Cosmic-Tools)
    TARGET_DIR="$PROJECT_ROOT/Deploy/$NS/bin"
    BINARY_NAME="${NS_LOWER}-server"
    echo "→ Building Gateway (namespace=$NS, features=${NS_LOWER},${SSO_FEATURE}, target=Deploy/$NS/target/)..."
    cd "$PROJECT_ROOT/Gateway/backend"
    BINARY_SRC="$PROJECT_ROOT/Deploy/$NS/target/$TARGET_DIR_SUFFIX/alioth-gateway"
    # 生产构建刻意不含 preproc-proxy feature（dev 形态专用）→ 产物无 /preproc/* 与
    # /api/pre_proc/* 未认证反代路由（404）。dev 任务（Gateway/backend/.mise.toml dev）
    # 显式追加 preproc-proxy 保持开发可用。见 openspec change fix-gateway-proxy-standalone-auth。
    cargo build $CARGO_FLAGS --manifest-path "$PROJECT_ROOT/Gateway/host/Cargo.toml" -p gateway-host --no-default-features --features "${NS_LOWER},${SSO_FEATURE}" --target-dir "$PROJECT_ROOT/Deploy/$NS/target"
    ;;
esac

# Ensure target dir exists
mkdir -p "$TARGET_DIR"

# Copy binary
check_binary_size_threshold "$BINARY_SRC"

# 原子替换：目标可能正被运行中实例执行（Linux 上对执行中的文件开写态 → ETXTBSY），
# rename 只换目录项，老进程继续持有旧 inode（无需停服）。
install_executable_atomic "$BINARY_SRC" "$TARGET_DIR/$BINARY_NAME"

# macOS 15+ provenance sandbox + 交付面剥符号（reduce-build-artifact-size）：
# ① 剥符号（本机 strip 默认档：去符号表，保留 __eh_frame 展开表）② 清理 provenance xattr
# ③ ad-hoc 重签——三步顺序固定，由原语统一实施（未 strip 的 Deploy/{ns}/target/release/ 副本保留）。
trim_release_binary "$TARGET_DIR/$BINARY_NAME"

echo "✅ Binary written: $TARGET_DIR/$BINARY_NAME"
echo "   Size: $(du -h "$TARGET_DIR/$BINARY_NAME" | cut -f1)"
echo ""

# ── 独立 SSO 产物（仅 SSO 形态 = remote）──────────────────────────────────
# remote 形态下 Gateway 不内嵌 SSO，认证 / PDP / JWKS 全走独立进程 ⇒ 独立 SSO MUST 与
# Gateway 同发布单元（Deploy/{ns}/bin/gateway-sso），否则发布包无法启动该 ns（启动器的
# SSO 前置进程无可执行）。与 Gateway 共用同一 target 目录 ⇒ 依赖编译缓存复用。
if [ "$SSO_FEATURE" = "sso-remote" ]; then
  echo "→ Building standalone SSO (形态=remote, target=Deploy/$NS/target/)..."
  cd "$PROJECT_ROOT"
  SSO_BINARY_SRC="$PROJECT_ROOT/Deploy/$NS/target/$TARGET_DIR_SUFFIX/gateway-sso"
  cargo build $CARGO_FLAGS -p gateway-sso --target-dir "$PROJECT_ROOT/Deploy/$NS/target"
  mkdir -p "$TARGET_DIR"
  check_binary_size_threshold "$SSO_BINARY_SRC"
  install_executable_atomic "$SSO_BINARY_SRC" "$TARGET_DIR/gateway-sso"
  trim_release_binary "$TARGET_DIR/gateway-sso"
  echo "✅ Standalone SSO written: $TARGET_DIR/gateway-sso"
  echo "   Size: $(du -h "$TARGET_DIR/gateway-sso" | cut -f1)"
  echo ""
fi

# ── OpenActivity（按需启用，Gateway 构建等位；refactor-openactivity-gateway-parity D15）──
# 启用判据 = Pre-Proc/{ns}/Open/Apps/*/app.json 存在（组合契约声明外部协同门户）；
# 未启用 ns 跳过（无 bin 产物、不创建 Open 运行面）。
if compgen -G "$PROJECT_ROOT/Pre-Proc/$NS/Open/Apps/*/app.json" >/dev/null; then
    echo "→ Building OpenActivity (opt-in, target=Deploy/$NS/target/)..."
    cd "$PROJECT_ROOT"
    cargo build $CARGO_FLAGS -p openactivity-server --target-dir "$PROJECT_ROOT/Deploy/$NS/target"
    # 落点 = Deploy/{ns}/Open/bin/（运行时归属布局：OpenActivity 管理 Deploy/{ns}/Open/，
    # 契约见 openspec/specs/external-entry 与 openactivity-runtime::gateway-parity-build）。
    # MUST NOT 写入 Deploy/{ns}/bin/（该目录归 Gateway 二进制）。
    OA_BIN="$PROJECT_ROOT/Deploy/$NS/Open/bin/openactivity-server"
    OA_BIN_SRC="$PROJECT_ROOT/Deploy/$NS/target/$TARGET_DIR_SUFFIX/openactivity-server"
    mkdir -p "$(dirname "$OA_BIN")"
    # 触发线检查 MUST 先于落盘（与 Gateway 分支同序）：超限产物不得进入交付目录
    check_binary_size_threshold "$OA_BIN_SRC"
    # 同 Gateway：原子替换（运行中 OpenActivity 实例持有旧 inode 时 cp 会 ETXTBSY）
    install_executable_atomic "$OA_BIN_SRC" "$OA_BIN"
    # 交付面剥符号 + provenance 清理 + ad-hoc 重签（同 Gateway，单一原语；顺序不可拆）
    trim_release_binary "$OA_BIN"

    echo "✅ OpenActivity binary written: $OA_BIN ($(du -h "$OA_BIN" | cut -f1))"

    # ── OpenActivity 前端（per-namespace 隔离落点 = Deploy/{ns}/Open/frontend）──
    # 门户静态面由此目录服务（release 只校验、不复制：共享 `OpenActivity/frontend/dist` 被多调用方
    # 覆写，从中复制会造成跨 ns 错配且不可检测）。
    # 依赖缺失（无 node_modules/pnpm，如纯 Rust 构建机）时**不阻断**：产物缺失由 release 校验 fail-closed。
    OA_FE_DIR="$PROJECT_ROOT/Deploy/$NS/Open/frontend"
    if command -v pnpm >/dev/null 2>&1; then
        echo "→ Building OpenActivity frontend (target=$OA_FE_DIR)..."
        if (cd "$PROJECT_ROOT" && \
            ALIOTH_FE_OUT_DIR="$OA_FE_DIR" \
            ALIOTH_FE_NAMESPACE="$NS" \
            ALIOTH_BUILD_COMMIT="$(git -C "$PROJECT_ROOT" rev-parse HEAD 2>/dev/null || echo '')" \
            pnpm --filter openactivity-frontend build); then
            echo "✅ OpenActivity frontend written: $OA_FE_DIR"
        else
            echo "❌ OpenActivity frontend 构建失败 — 发布前必须修复（openactivity-runtime::gateway-parity-build）"
            echo "    重试: ALIOTH_FE_OUT_DIR=$OA_FE_DIR ALIOTH_FE_NAMESPACE=$NS pnpm --filter openactivity-frontend build"
            exit 1
        fi
    else
        echo "⚠️  未找到 pnpm — 跳过 OpenActivity 前端构建（release 校验会 fail-closed）"
    fi
else
    echo "⏭  OpenActivity 未启用（无 Pre-Proc/$NS/Open/Apps/*/app.json）— 跳过"
fi

# apps.json 聚合（APP_EXTENSION §4.5）：生产模式 App 发现数据源，随包生成
# 聚合器 = `scripts/gen-apps-json.ts`（Bun）——**不用 jq**：本机 jq 可能是 jaq，对多输入文件
# 逐个输出 ⇒ ≥2 个 app 时写出多个顶层文档，Gateway `parse_apps_json` 解析失败、App 发现为空
# （2026-09-23 实测 Alioth 2 app）
DEPLOY_ROOT="$PROJECT_ROOT/Deploy/$NS"
if [ -d "$PROJECT_ROOT/Pre-Proc/$NS/Apps" ]; then
    if bun "$PROJECT_ROOT/scripts/gen-apps-json.ts" "$PROJECT_ROOT/Pre-Proc/$NS/Apps" "$DEPLOY_ROOT/apps.json"; then
        :
    else
        echo "⚠️  apps.json 生成失败"
    fi
fi

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Done. Start with:"
echo "    cd Deploy/$NS && ./bin/$BINARY_NAME"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
