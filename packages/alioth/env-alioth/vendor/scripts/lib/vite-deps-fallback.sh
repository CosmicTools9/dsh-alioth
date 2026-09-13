#!/usr/bin/env bash
# vite-deps-fallback.sh — vite deps 优化器卡死检测 + 自动修复（dev FE 白屏自愈，共享）
#
# 供给方：scripts/dev/dev-gateway.sh（Gateway FE）、scripts/dev/dev-openactivity.sh（OpenActivity FE）
# 背景（2026-09-03 白屏事故）：内嵌优化器偶发永不完成（.vite/deps 空、deps URL 504 → 白屏）；
# 手动 `npx vite optimize` 实测秒级正常——检测到即补产物并重启 FE 复用缓存。
#
# 用法（调用方需先设 PROJECT_ROOT；FE 目录/端口由参数给）:
#   source scripts/lib/vite-deps-fallback.sh
#   fe_deps_probe_url <port>                       # 输出 deps 引用（无引用输出空）
#   fe_deps_fallback <fe_dir> <port> <log_file> [pid_file] [pid_key]
#     0 = 无需修复或已修复；1 = 修复失败
#     修复成功时导出 FE_DEPS_NEW_PID（新 vite PID）；给了 pid_file+pid_key 时同步回写
#   fe_pid_file_set <file> <key> <value>           # 便携回写 KEY=value（无 sed 方言依赖）

# 从 main.tsx 模块引用抓带版本 hash 的 deps URL（无引用返回空）
fe_deps_probe_url() {
    local port="$1"
    curl -s --noproxy '*' -m 3 "http://127.0.0.1:${port}/src/main.tsx" 2>/dev/null \
        | grep -oE 'deps/[a-z0-9_.-]+\.js\?v=[a-f0-9]+' | head -1
}

# 便携回写 pid 文件里的 KEY=<pid> 行（无 sed -i 方言依赖）
fe_pid_file_set() {
    local file="$1" key="$2" value="$3" tmp line found=0
    [ -f "$file" ] || return 1
    tmp="${file}.tmp.$$"
    : > "$tmp" || return 1
    while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in
            "${key}="*) printf '%s=%s\n' "$key" "$value" >> "$tmp"; found=1 ;;
            *) printf '%s\n' "$line" >> "$tmp" ;;
        esac
    done < "$file"
    if [ "$found" = 0 ]; then
        printf '%s=%s\n' "$key" "$value" >> "$tmp"
    fi
    mv -f "$tmp" "$file"
}

# fe_deps_count <fe_dir> —— .vite/deps 下 .js 产物数（0 表示优化器未产出）
fe_deps_count() {
    local dir="$1/node_modules/.vite/deps" f n=0
    for f in "$dir"/*.js; do
        [ -e "$f" ] && n=$((n + 1))
    done
    echo "$n"
}

# fe_deps_http_code <url>
fe_deps_http_code() {
    curl -s --noproxy '*' -o /dev/null -m 3 -w '%{http_code}' "$1" 2>/dev/null || true
}

# fe_deps_fallback <fe_dir> <port> <log_file> [pid_file] [pid_key]
fe_deps_fallback() {
    local fe_dir="$1" port="$2" log_file="$3" pid_file="${4:-}" pid_key="${5:-}"
    local ref code n root_code new_pid
    ref="$(fe_deps_probe_url "$port")"
    n="$(fe_deps_count "$fe_dir")"
    if [ -n "$ref" ]; then
        code="$(fe_deps_http_code "http://127.0.0.1:${port}/node_modules/.vite/${ref}")"
        [ "$code" = "200" ] && return 0
    fi
    # 判卡死（盘面事实优先）：root 可达 && deps 目录零产物 → 优化器永不完成；
    # ref 非空但非 200 同判。ref 空 + root 不可达 = FE 未就绪（交调用方就绪等待报错）
    root_code="$(fe_deps_http_code "http://127.0.0.1:${port}/")"
    [ "$root_code" != "200" ] && return 0
    if [ "${n:-0}" -eq 0 ] || [ -n "$ref" ]; then
        echo -e "\033[1;33m[vite-fallback]\033[0m deps 优化器卡死（产物 ${n:-0} 个，${ref:-无引用} → HTTP ${code:-—}），手动 optimize + 重启 FE…"
    else
        return 0
    fi
    # 先杀 dev server（占锁会干扰 optimize），optimize 秒级产出缓存，重启即复用不再走内嵌优化
    local holder
    holder="$(lsof -ti :"$port" 2>/dev/null || true)"
    # shellcheck disable=SC2086 # 有意按空白切分多个 PID
    [ -n "$holder" ] && kill $holder 2>/dev/null || true
    sleep 1
    (cd "$fe_dir" && npx vite optimize >/dev/null 2>&1) || return 1
    n="$(fe_deps_count "$fe_dir")"
    if [ "${n:-0}" -eq 0 ]; then
        echo -e "\033[0;31m[vite-fallback]\033[0m optimize 后仍无 deps 产物"
        return 1
    fi
    (cd "$fe_dir" && nohup ${FE_DEPS_START_CMD:-bun run dev -- --port "$port"} > "$log_file" 2>&1 &) || return 1
    sleep 1
    new_pid="$(lsof -ti :"$port" 2>/dev/null | head -1 || true)"
    # 输出契约：调用方据此更新自身 FE PID 变量/pid 文件
    # shellcheck disable=SC2034
    FE_DEPS_NEW_PID="$new_pid"
    [ -n "$pid_file" ] && [ -n "$pid_key" ] && [ -n "$new_pid" ] && fe_pid_file_set "$pid_file" "$pid_key" "$new_pid"
    # 重启后就绪等待（冷启动 + 缓存复用 <2s；无等待会探测过早误报）
    ref=""
    for _ in $(seq 1 10); do
        ref="$(fe_deps_probe_url "$port")"
        [ -n "$ref" ] && break
        sleep 1
    done
    if [ -z "$ref" ]; then
        echo -e "\033[0;31m[vite-fallback]\033[0m 重启后无 deps 引用（FE 未就绪）"
        return 1
    fi
    code="$(fe_deps_http_code "http://127.0.0.1:${port}/node_modules/.vite/${ref}")"
    if [ "$code" = "200" ]; then
        echo -e "\033[0;32m[vite-fallback]\033[0m deps 已修复（HTTP 200，FE PID ${new_pid:-?}）"
        return 0
    fi
    echo -e "\033[0;31m[vite-fallback]\033[0m 重启后仍 ${code}——白屏风险，查 ${log_file}"
    return 1
}
