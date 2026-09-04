#!/bin/bash
# build-ns.sh — 按 namespace 编译 standalone binary 并输出到 Deploy/{ns}/bin/
#
# 用法:
#   bash scripts/build-ns.sh WZ [profile]
#   bash scripts/build-ns.sh Alioth [profile]
#   bash scripts/build-ns.sh AVIC-CAASEC [profile]
#   bash scripts/build-ns.sh Meta [profile]
#
# profile 默认 release，可指定 dev
#
# Framework/backend/   ← 根 workspace：共享基础设施 + SSO + Gateway
# Pre-Proc/{ns}/       ← 独立 workspace，各自有独立的 Cargo.lock + target 目录
# 构建 gateway binary：cd Gateway/backend && cargo build -p alioth-gateway --features {ns},sso
# Service crates 跨 workspace 通过 path deps 引用，Cargo 自动解析。
# 日常开发：cd Pre-Proc/{ns} && cargo check -p {ns}-service-xxx

set -euo pipefail

NS="${1:?Usage: build-ns.sh <namespace> [profile]}"
PROFILE="${2:-release}"
NS_LOWER="$(echo "$NS" | tr '[:upper:]' '[:lower:]')"
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# 触发线检测（add-build-threshold-check）：二进制 >500MB → 失败（时间线已移除）
source "$PROJECT_ROOT/scripts/lib/build-thresholds.sh"

if [ "$PROFILE" = "release" ]; then
  CARGO_FLAGS="--release"
  TARGET_DIR_SUFFIX="release"
  # sccache: 全局通过 .cargo/config.toml 启用
  # release: 关闭 incremental（只缓存全量编译产物）
  export CARGO_PROFILE_RELEASE_INCREMENTAL=false
  echo "  sccache: 已启用 (local disk, max 30 GiB)"
else
  CARGO_FLAGS=""
  TARGET_DIR_SUFFIX="debug"
fi

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Building standalone binary for namespace: $NS"
echo "  Profile: $PROFILE"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"


case "$NS" in
  Meta)
    TARGET_DIR="$PROJECT_ROOT/Deploy/Meta/bin"
    BINARY_NAME="meta-server"
    echo "→ Building Meta backend (workspace member)..."
    # Meta/backend 是独立 workspace（根 workspace 不含 Meta/*）——须在该目录构建，
    # 产物位于 Meta/backend/target/（而非根 target/）
    cd "$PROJECT_ROOT/Meta/backend"
    BINARY_SRC="$PROJECT_ROOT/Meta/backend/target/$TARGET_DIR_SUFFIX/meta-backend"
    cargo build $CARGO_FLAGS -p meta-backend
    ;;
  Alioth|WZ|AVIC-CAASEC)
    TARGET_DIR="$PROJECT_ROOT/Deploy/$NS/bin"
    BINARY_NAME="${NS_LOWER}-server"
    echo "→ Building Gateway (features=$NS, target=Deploy/$NS/target/)..."
    cd "$PROJECT_ROOT/Gateway/backend"
    BINARY_SRC="$PROJECT_ROOT/Deploy/$NS/target/$TARGET_DIR_SUFFIX/alioth-gateway"
    # 生产构建刻意不含 preproc-proxy feature（dev 形态专用）→ 产物无 /preproc/* 与
    # /api/pre_proc/* 未认证反代路由（404）。dev 任务（Gateway/backend/.mise.toml dev）
    # 显式追加 preproc-proxy 保持开发可用。见 openspec change fix-gateway-proxy-standalone-auth。
    cargo build $CARGO_FLAGS -p alioth-gateway --no-default-features --features "$NS_LOWER,sso" --target-dir "$PROJECT_ROOT/Deploy/$NS/target"
    NEEDS_RESIGN=true
    ;;
  Cosmic-Tools)
    TARGET_DIR="$PROJECT_ROOT/Deploy/Cosmic-Tools/bin"
    BINARY_NAME="cosmic-tools-server"
    echo "→ Building Gateway (features=cosmic-tools, target=Deploy/Cosmic-Tools/target/)..."
    cd "$PROJECT_ROOT/Gateway/backend"
    BINARY_SRC="$PROJECT_ROOT/Deploy/Cosmic-Tools/target/$TARGET_DIR_SUFFIX/alioth-gateway"
    # 同 Alioth/WZ/AVIC 分支：生产构建不含 preproc-proxy（未认证反代仅 dev 可用）
    cargo build $CARGO_FLAGS -p alioth-gateway --no-default-features --features "cosmic-tools,sso" --target-dir "$PROJECT_ROOT/Deploy/Cosmic-Tools/target"
    NEEDS_RESIGN=true
    ;;
  *)
    # Generic namespace fallback (dsh-alioth): any namespace whose
    # {ns_lower} feature exists in the gateway manifest builds here.
    TARGET_DIR="$PROJECT_ROOT/Deploy/$NS/bin"
    BINARY_NAME="${NS_LOWER}-server"
    echo "→ Building Gateway (features=$NS_LOWER, target=Deploy/$NS/target/)..."
    cd "$PROJECT_ROOT/Gateway/backend"
    BINARY_SRC="$PROJECT_ROOT/Deploy/$NS/target/$TARGET_DIR_SUFFIX/alioth-gateway"
    cargo build $CARGO_FLAGS -p alioth-gateway --no-default-features --features "$NS_LOWER,sso" --target-dir "$PROJECT_ROOT/Deploy/$NS/target"
    NEEDS_RESIGN=true
    ;;
esac

# Ensure target dir exists
mkdir -p "$TARGET_DIR"

# Copy binary
check_binary_size_threshold "$BINARY_SRC"

cp "$BINARY_SRC" "$TARGET_DIR/$BINARY_NAME"
chmod +x "$TARGET_DIR/$BINARY_NAME"

# macOS 15+ provenance sandbox: strip xattr and re-sign ad-hoc.
# cp between filesystems on Sequoia sets com.apple.provenance xattr, which
# causes the kernel to silently kill the binary (exit 137) before main().
# Only needed for Gateway standalone binaries (non-Meta namespaces).
xattr -d com.apple.provenance "$TARGET_DIR/$BINARY_NAME" 2>/dev/null || true
codesign --force --deep --sign - "$TARGET_DIR/$BINARY_NAME" 2>/dev/null || true

echo "✅ Binary written: $TARGET_DIR/$BINARY_NAME"
echo "   Size: $(du -h "$TARGET_DIR/$BINARY_NAME" | cut -f1)"
echo ""

# ── OpenActivity（按需启用，Gateway 构建等位；refactor-openactivity-gateway-parity D15）──
# 启用判据 = Pre-Proc/{ns}/Open/Apps/*/app.json 存在（组合契约声明外部协同门户）；
# 未启用 ns 跳过（无 bin 产物、不创建 Open 运行面）。
if compgen -G "$PROJECT_ROOT/Pre-Proc/$NS/Open/Apps/*/app.json" >/dev/null; then
    echo "→ Building OpenActivity (opt-in, target=Deploy/$NS/target/)..."
    cd "$PROJECT_ROOT"
    cargo build $CARGO_FLAGS -p openactivity-server --target-dir "$PROJECT_ROOT/Deploy/$NS/target"
    OA_BIN="$TARGET_DIR/openactivity-server"
    cp "$PROJECT_ROOT/Deploy/$NS/target/$TARGET_DIR_SUFFIX/openactivity-server" "$OA_BIN"
    chmod +x "$OA_BIN"
    # 同 Gateway：macOS provenance xattr 清理 + ad-hoc 签名
    xattr -d com.apple.provenance "$OA_BIN" 2>/dev/null || true
    codesign --force --deep --sign - "$OA_BIN" 2>/dev/null || true
    check_binary_size_threshold "$OA_BIN"

    echo "✅ OpenActivity binary written: $OA_BIN ($(du -h "$OA_BIN" | cut -f1))"
else
    echo "⏭  OpenActivity 未启用（无 Pre-Proc/$NS/Open/Apps/*/app.json）— 跳过"
fi

# apps.json 聚合（APP_EXTENSION §4.5）：生产模式 App 发现数据源，随包生成
DEPLOY_ROOT="$PROJECT_ROOT/Deploy/$NS"
if command -v jq >/dev/null 2>&1 && compgen -G "$PROJECT_ROOT/Pre-Proc/$NS/Apps/*/app.json" >/dev/null; then
    if jq -n '{appInstances: [inputs]}' "$PROJECT_ROOT"/Pre-Proc/"$NS"/Apps/*/app.json > "$DEPLOY_ROOT/apps.json" 2>/dev/null; then
        echo "✅ apps.json written: $DEPLOY_ROOT/apps.json ($(jq '.appInstances | length' "$DEPLOY_ROOT/apps.json") apps)"
    else
        echo "⚠️  apps.json 生成失败"
    fi
elif [ -d "$PROJECT_ROOT/Pre-Proc/$NS/Apps" ]; then
    echo '{"appInstances":[]}' > "$DEPLOY_ROOT/apps.json"
    echo "⚠️  无 Apps/*/app.json 或 jq 缺失 — apps.json 空聚合"
fi

echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  Done. Start with:"
echo "    cd Deploy/$NS && ./bin/$BINARY_NAME"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
