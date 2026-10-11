#!/bin/bash
# build-ns.sh — 按 namespace 编译 standalone binary 并输出到 Deploy/{ns}/bin/
#
# 用法:
#   bash scripts/build-ns.sh WZ [profile] [--clean] [--target-root <dir>]
#   bash scripts/build-ns.sh Alioth [profile]
#   bash scripts/build-ns.sh AVIC-CAASEC [profile]
#
# --target-root <dir>（可选、缺省 = 现状行为）：覆写**产物落点根**（默认 ${PROJECT_ROOT}/Deploy/{ns}）。
#   给定值时产物（bin/、Open/、apps.json）落 <dir>，**不触碰 live Deploy/{ns}**——供隔离链在暂存
#   副本上构建（与 release-to-namespace.sh --deploy-root 传同一 <dir>）。
#
# profile 默认 release，可指定 dev
# --clean 在构建前移除**共享依赖层**（全 ns 共用；本次将全量重建，影响所有 ns 的下次构建）；
#         MUST NOT 触碰 Deploy/{ns}/bin/（运行中实例可能持有该二进制）
#
# Meta 不是 namespace，不接受本脚本的参数：其权威构建 = scripts/meta/build.sh
# （产物 Meta/backend/target/），打包 = scripts/meta/deploy.sh --target Release/Meta。
#
# Framework/backend/   ← 根 workspace：共享基础设施 + SSO + Gateway
# Pre-Proc/{ns}/       ← 独立 workspace，各自有独立的 Cargo.lock + target 目录
# 构建 gateway binary：build-ns.sh 在 Gateway/backend 下执行
#   cargo build --manifest-path Gateway/host/Cargo.toml -p gateway-host --no-default-features --features {ns},sso --target-dir "$(bash scripts/cargo-target.sh ns {ns})"
# Service crates 跨 workspace 通过 path deps 引用，Cargo 自动解析。
# 日常开发：cd Pre-Proc/{ns} && cargo check -p {ns}-service-xxx

set -euo pipefail

# 参数解析：位置参数 <namespace> [profile] + 可选旗标 --clean / --target-root <dir>（位置无关）
CLEAN=false
TARGET_ROOT_OVERRIDE=""
POSITIONAL=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --clean) CLEAN=true ;;
    --target-root)
      shift
      TARGET_ROOT_OVERRIDE="${1:-}"
      if [ -z "$TARGET_ROOT_OVERRIDE" ]; then
        echo "❌ --target-root 需要一个目录参数" >&2
        exit 1
      fi
      ;;
    *) POSITIONAL+=("$1") ;;
  esac
  shift
done

NS="${POSITIONAL[0]:?Usage: build-ns.sh <namespace> [profile] [--clean] [--target-root <dir>]}"
PROFILE="${POSITIONAL[1]:-release}"
PROJECT_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# ── 磁盘阈值门禁（change enforce-deploy-surface-convergence :: deploy-artifact-recycling-and-disk-threshold）──
# 可用空间 < 阈值（默认 5G）时拒绝启动并打印清理面清单——防构建半途 ENOSPC（2026-10-01 盒实证：磁盘 91%）。
# 覆写：ALIOTH_MIN_FREE_GB=<n>（阈值，G）；ALIOTH_SKIP_DISK_GATE=1（跳过，仅排障用）。
check_free_disk() {
  if [ "${ALIOTH_SKIP_DISK_GATE:-0}" = "1" ]; then
    echo "→ 磁盘门禁：已跳过（ALIOTH_SKIP_DISK_GATE=1）"
    return 0
  fi
  local min_gb="${ALIOTH_MIN_FREE_GB:-5}"
  local avail_kb
  # df -Pk：POSIX 输出（1024 块）；数据行第 4 列 = available（macOS/Linux 同形）。
  avail_kb="$(df -Pk "$PROJECT_ROOT" 2>/dev/null | awk 'NR==2 {print $4}')"
  case "${avail_kb}" in ''|*[!0-9]*) echo "→ 磁盘门禁：无法解析可用空间（跳过检查）"; return 0 ;; esac
  local min_kb=$((min_gb * 1024 * 1024))
  if [ "${avail_kb}" -lt "${min_kb}" ]; then
    echo "❌ 磁盘可用空间不足：$((avail_kb / 1024 / 1024))G < ${min_gb}G（阈值）——拒绝启动构建" >&2
    echo "   清理面清单（按收益排序）：" >&2
    echo "     ① 平台根编译缓存：rm -rf ${PROJECT_ROOT}/target ${PROJECT_ROOT}/Meta/backend/target" >&2
    echo "     ② 共享依赖层（ns 构建缓存）：bash scripts/cargo-target.sh ns ${NS:-<ns>} 得落点后 rm -rf <落点>" >&2
    echo "     ③ /tmp 大件（>100M 且 >7 天）：bash scripts/deploy/reclaim-tmp.sh" >&2
    echo "     ④ sccache 缓存：sccache --stop-server && rm -rf \${SCCACHE_DIR:-~/.cache/sccache}" >&2
    echo "     ⑤ pnpm store：pnpm store prune" >&2
    echo "   （确需绕过：ALIOTH_SKIP_DISK_GATE=1 bash scripts/build-ns.sh …）" >&2
    exit 1
  fi
  echo "→ 磁盘门禁：可用 $((avail_kb / 1024 / 1024))G ≥ ${min_gb}G ✓"
}
check_free_disk

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
# `${NS:-alioth}`；不归一时 `Deploy/alioth/*` 会与 `Deploy/Alioth/*` 分裂出两套部署面
# （大小写契约 = docs/specs/REPO_LAYOUT_SPEC.md §名称大小写契约；判据 = check-layout-name-case）。
source "$PROJECT_ROOT/scripts/lib/ns-name.sh"
NS="$(ns_canon "$NS")"
NS_LOWER="$(ns_lower "$NS")"

# ns 构建产物落点（唯一映射 = scripts/lib/cargo-target-dirs.sh 的 `ns` 用途）。2026-10-04 用户裁决
# 起该用途为**共享依赖层**（全 ns 同目录 ⇒ 同源依赖图只编译一份，重开 ADR D-026 作用域）；
# `CARGO_TARGET_DIR_NS` 覆写可回到 per-ns 隔离。本脚本 MUST 经此变量取落点、MUST NOT 内联裸路径
# （判据 = scripts/check/check-ns-build-target-single-source.ts）。
source "$PROJECT_ROOT/scripts/lib/cargo-target-dirs.sh"
NS_BUILD_TARGET_DIR="$(cargo_target_dir ns "$NS")"

# Deploy/{ns} 归属解析（单一实现 = scripts/lib/ns-deploy-dir.sh）：拆分后 Deploy/{ns} 归 ns 仓，
# 平台仓缺该目录时自动建立相对符号链接 → ../Alioth-NS-<ns>/Deploy/<ns>；否则本脚本会在平台仓
# 另建一份空 Deploy/{ns}（缺 .env/apps.json/Apps）⇒ 产物与运行面分叉（2026-09-25 实证）。
# --target-root <dir> 覆写时**不触碰 live Deploy/{ns}**（隔离链：在 <dir> 下落产物，供暂存副本构建）。
DEPLOY_NS_DIR="$PROJECT_ROOT/Deploy/$NS"
if [ -n "$TARGET_ROOT_OVERRIDE" ]; then
    DEPLOY_NS_DIR="$TARGET_ROOT_OVERRIDE"
    mkdir -p "$DEPLOY_NS_DIR"
    echo "→ --target-root：产物落隔离根 ${DEPLOY_NS_DIR}（不触碰 ${PROJECT_ROOT}/Deploy/${NS}）"
else
    if [ -f "$PROJECT_ROOT/scripts/lib/ns-deploy-dir.sh" ]; then
        # shellcheck source=scripts/lib/ns-deploy-dir.sh
        # shellcheck disable=SC1091
        source "$PROJECT_ROOT/scripts/lib/ns-deploy-dir.sh"
        ns_deploy_dir_ensure "$NS"
    fi
fi

# ── 兼容落点：`Deploy/{ns}/target` = **指向共享依赖层的符号链接** ─────────────
# 为什么保留该路径：多处既有调用点（dev/prod/pre start、restart-gateway、dev-openactivity、
# env-common、promote、manifest…）以 `Deploy/{ns}/target` 表达「该 ns 的构建缓存」。共享层上线后
# 若它们仍写该路径，会重新分裂出 per-ns 依赖图（正是本次要消除的）。做成软链 ⇒ 全部调用点无须
# 改动即落同一物理目录，「单一落点」语义成立且落点可自证（realpath 相同）。
# 自愈两种遗留态：① 真目录（旧 per-ns 缓存，忽略态/可再生）⇒ 移除并打印；② 指向他处的旧链接 ⇒ 重指。
# --target-root 隔离链下跳过（隔离根无 legacy `target` 消费者；不改动 live 面）。
NS_TARGET_LINK="$DEPLOY_NS_DIR/target"
if [ -z "$TARGET_ROOT_OVERRIDE" ] && [ -e "$NS_TARGET_LINK" ] && [ ! -L "$NS_TARGET_LINK" ]; then
  echo "→ 迁移遗留 per-ns 编译缓存：移除 ${NS_TARGET_LINK}（$(du -sh "${NS_TARGET_LINK}" 2>/dev/null | cut -f1)）；构建缓存已由共享依赖层接管"
  rm -rf "$NS_TARGET_LINK"
elif [ -z "$TARGET_ROOT_OVERRIDE" ] && [ -L "$NS_TARGET_LINK" ] && [ "$(readlink "${NS_TARGET_LINK}")" != "$NS_BUILD_TARGET_DIR" ]; then
  echo "→ 重指构建缓存链接：$(readlink "${NS_TARGET_LINK}") → ${NS_BUILD_TARGET_DIR}"
  rm -f "$NS_TARGET_LINK"
fi
if [ -z "$TARGET_ROOT_OVERRIDE" ] && [ ! -L "$NS_TARGET_LINK" ]; then
  mkdir -p "$(dirname "${NS_TARGET_LINK}")" "$NS_BUILD_TARGET_DIR"
  # 绝对链接：目标是仓外共享层（~/.tmp），相对链接在 Deploy/{ns} 归属迁移后易脆。
  ln -sfn "$NS_BUILD_TARGET_DIR" "$NS_TARGET_LINK"
  echo "→ 构建缓存落点：${NS_TARGET_LINK} -> ${NS_BUILD_TARGET_DIR}（共享依赖层）"
fi

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

# ── 可选：清理编译缓存（--clean）───────────────────────────────────────────
# 清理目标 = **共享依赖层**（`cargo_target_dir ns` 的落点；2026-10-04 裁决后全 ns 共用）
# ⇒ 本操作**影响所有 ns 的下次构建**（各 ns 定向 crate 将重编；同源依赖与第三方由 sccache
#   与热缓存吸收）。需要只清某个 ns 的 per-ns 隔离缓存时，先用
#   `CARGO_TARGET_DIR_NS=<root>/Deploy/<ns>/target` 覆写构建，再手工清理该目录。
# MUST NOT 触 bin/：运行中实例可能持有 Deploy/{ns}/bin/{ns}-server，删除会中断服务，
# 且无必要——产物由随后的构建替换。
# 路径取自唯一映射 lib（非外部输入）；NS 已经 ns_canon 归一 + 上方白名单校验。
# 位置 MUST 在构建之前：本脚本的构建分支自带 cargo build，清理若后置会删掉刚产出的二进制。
if [ "$CLEAN" = true ]; then
  NS_TARGET_DIR="$NS_BUILD_TARGET_DIR"
  # 变量一律用 ${VAR} 花括号边界：裸 $VAR 紧邻全角字符（如「（」）时，bash 在 set -u 下
  # 会把全角字符的首字节并入变量名 → `unbound variable`（本仓已两次踩到，见
  # fix(pre-push) 的同名缺陷）。MUST NOT 改写为裸 $-形态。
  if [ -d "${NS_TARGET_DIR}" ]; then
    echo "→ --clean: 移除共享依赖层 ${NS_TARGET_DIR} ($(du -sh "${NS_TARGET_DIR}" 2>/dev/null | cut -f1))——本次将全量重建（影响所有 ns 的下次构建）"
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

# ── 组合根来源（move-composition-root-to-ns；平台回落分支已退役 2026-10-06）─────────────
# 唯一组合根 = ns 自持 Pre-Proc/{ns}/gateway-host（default = 本 ns 服务集，仅 --features
# {sso}，bin 名 {ns}-server，build.rs 扫描根 MUST 显式传 SERVICES_SOURCE_PATH）。
# auto（默认）= ns host 存在即走 ns；缺失即 fail-loud（先实例化，无从回退——平台模板的
# 裸 ns feature 桩先期已删，旧 platform 回落路径必报未知 feature，故退役）。
HOST_ORIGIN="${ALIOTH_HOST_ORIGIN:-auto}"
NS_HOST_MANIFEST="$PROJECT_ROOT/Pre-Proc/$NS/gateway-host/Cargo.toml"
if [ "$HOST_ORIGIN" != "auto" ] && [ "$HOST_ORIGIN" != "ns" ]; then
  echo "❌ ALIOTH_HOST_ORIGIN=${HOST_ORIGIN} 不受支持：平台回落分支已退役（2026-10-06，5/5 ns 已迁移）。" >&2
  echo "   唯一组合根 = ns 自持 Pre-Proc/{ns}/gateway-host（用 auto 或缺省即可）。" >&2
  exit 1
fi
if [ ! -f "$NS_HOST_MANIFEST" ]; then
  echo "❌ 缺 ${NS_HOST_MANIFEST}——ns 组合根未实例化（平台回落已退役，无从回退）" >&2
  echo "   先实例化：bun scripts/gateway/instantiate-ns-host.ts --ns $NS --repo <ns 仓根>" >&2
  exit 1
fi

# ── 构建 ────────────────────────────────────────────────────────────────
case "$NS" in
  Alioth|WZ|AVIC-CAASEC|SE|Cosmic-Tools)
    TARGET_DIR="$DEPLOY_NS_DIR/bin"
    BINARY_NAME="${NS_LOWER}-server"
    echo "→ Building Gateway (origin=ns, namespace=$NS, features=${SSO_FEATURE}, target=$NS_BUILD_TARGET_DIR)..."
    cd "$PROJECT_ROOT"
    BINARY_SRC="$NS_BUILD_TARGET_DIR/$TARGET_DIR_SUFFIX/${NS_LOWER}-server"
    # ns host 的 build.rs 扫描根 MUST 显式传入（默认相对路径在 ns 布局下 fail-closed panic）
    export SERVICES_SOURCE_PATH="$PROJECT_ROOT/Pre-Proc"
    # 服务集变化（新增/删除服务目录）由模板 build.rs 的目录级 rerun-if-changed 覆盖
    # （2026-09-26 注入）；service.json 内容变化由文件级 rerun 覆盖——无需显式失效。
    cargo build $CARGO_FLAGS --manifest-path "$NS_HOST_MANIFEST" -p gateway-host --features "${SSO_FEATURE}" --target-dir "$NS_BUILD_TARGET_DIR"
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
# ③ ad-hoc 重签——三步顺序固定，由原语统一实施（未 strip 的 <共享依赖层>/release/ 副本保留）。
trim_release_binary "$TARGET_DIR/$BINARY_NAME"

echo "✅ Binary written: $TARGET_DIR/$BINARY_NAME"
echo "   Size: $(du -h "$TARGET_DIR/$BINARY_NAME" | cut -f1)"
echo ""

# ── 独立 SSO 产物（仅 SSO 形态 = remote）──────────────────────────────────
# remote 形态下 Gateway 不内嵌 SSO，认证 / PDP / JWKS 全走独立进程 ⇒ 独立 SSO MUST 与
# Gateway 同发布单元（Deploy/{ns}/bin/gateway-sso），否则发布包无法启动该 ns（启动器的
# SSO 前置进程无可执行）。与 Gateway 共用同一 target 目录 ⇒ 依赖编译缓存复用。
if [ "$SSO_FEATURE" = "sso-remote" ]; then
  echo "→ Building standalone SSO (形态=remote, target=$NS_BUILD_TARGET_DIR)..."
  cd "$PROJECT_ROOT"
  SSO_BINARY_SRC="$NS_BUILD_TARGET_DIR/$TARGET_DIR_SUFFIX/gateway-sso"
  # ns host 承载 gateway-sso bin（required-features=sso-remote，src 零副本指 SSO/backend）
  cargo build $CARGO_FLAGS --manifest-path "$NS_HOST_MANIFEST" --bin gateway-sso --features sso-remote --target-dir "$NS_BUILD_TARGET_DIR"
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
    echo "→ Building OpenActivity (opt-in, target=$NS_BUILD_TARGET_DIR)..."
    cd "$PROJECT_ROOT"
    # ns host 承载 openactivity-server bin（required-features=openactivity，零副本指 OpenActivity/backend）
    cargo build $CARGO_FLAGS --manifest-path "$NS_HOST_MANIFEST" --bin openactivity-server --features openactivity --target-dir "$NS_BUILD_TARGET_DIR"
    # 落点 = Deploy/{ns}/Open/bin/（运行时归属布局：OpenActivity 管理 Deploy/{ns}/Open/，
    # 契约见 openspec/specs/external-entry 与 openactivity-runtime::gateway-parity-build）。
    # MUST NOT 写入 Deploy/{ns}/bin/（该目录归 Gateway 二进制）。
    OA_BIN="$DEPLOY_NS_DIR/Open/bin/openactivity-server"
    OA_BIN_SRC="$NS_BUILD_TARGET_DIR/$TARGET_DIR_SUFFIX/openactivity-server"
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
    OA_FE_DIR="$DEPLOY_NS_DIR/Open/frontend"
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

# ── OpenAPI 文档产物（供给链：构建期重生成；发布包随包交付）──────────────────────
# 生成逻辑单源 = scripts/lib/openapi-doc.ts（CLI 壳 scripts/openapi-generate.ts）。
# 产物落 Pre-Proc/{ns}/openapi.json = 真相源（提交归 ns 仓）；发布期由
# release-to-namespace.sh 显式复制进 Deploy/{ns}/openapi.json（Gateway DEPLOY_PATH 语义读该路径）。
# 新鲜度/缺失由门禁 check-openapi-doc-drift 拦截；构建期失败（幽灵路径 / 非法动作声明）即
# fail-loud（set -e），MUST NOT 静默把陈旧/缺失文档带进发布包。
echo "→ Generating OpenAPI doc (Pre-Proc/$NS/openapi.json)..."
bun "$PROJECT_ROOT/scripts/openapi-generate.ts" --ns "$NS"

# apps.json 聚合（APP_EXTENSION §4.5）：生产模式 App 发现数据源，随包生成
# 聚合器 = `scripts/gen-apps-json.ts`（Bun）——**不用 jq**：本机 jq 可能是 jaq，对多输入文件
# 逐个输出 ⇒ ≥2 个 app 时写出多个顶层文档，Gateway `parse_apps_json` 解析失败、App 发现为空
# （2026-09-23 实测 Alioth 2 app）
DEPLOY_ROOT="$DEPLOY_NS_DIR"
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
