/**
 * `@dsh-alioth/auth-web-alioth` — client face (browser half), HAND-AUTHORED
 * closure-factory artifact for the harness client module system.
 *
 * Why hand-authored: the harness's `clientBundle` tsdown preset is not
 * published, and this module needs no bundling — one React component, zero
 * imports beyond the platform `react` module (resolved through the injected
 * `require`, per the lazy-CJS module table contract). Keep this file
 * dependency-free and side-effect-free outside the factory closure: executing
 * the script only REGISTERS the factory; everything else runs at
 * materialization.
 *
 * What it does: registers the user chip into the frame-wide `shell.overlay`
 * list slot (the documented additive seat for status pills). Identity comes
 * from the same-origin `/api/auth/me` (HttpOnly session cookie); logout posts
 * `/api/auth/logout` (clears cookies server-side) and bounces to /landing.
 * A 401 renders the chip in its signed-out form (登录 / 首页) instead of
 * vanishing: the console cookie outlives the Alioth session, so an expired
 * session must never leave the user with no way to sign in or out.
 */
window.__ModuleLoader__.load({
  id: '@dsh-alioth/auth-web-alioth',
  factory(require) {
    const module = { exports: {} }
    const exports = module.exports
    const React = require('react')
    const e = React.createElement

    // The overlay layer is click-through by design; the chip opts back in.
    const styles = {
      // Bottom-right: the frame's top-right hosts conversation actions
      // (export etc.) — the overlay must not cover them.
      chip: {
        position: 'fixed', bottom: 12, right: 12, zIndex: 99999,
        display: 'flex', alignItems: 'center', gap: 8,
        background: '#101724', border: '1px solid #1e2a3a', borderRadius: 999,
        padding: '4px 6px 4px 12px', font: '13px system-ui', color: '#d7e0ea',
        boxShadow: '0 2px 10px rgba(0,0,0,.4)', pointerEvents: 'auto',
      },
      name: { fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace', color: '#3ee6a8' },
      namespace: { color: '#7d8ca0', fontSize: 12 },
      link: { color: '#4fc3f7', fontSize: 12, textDecoration: 'none', padding: '0 2px' },
      button: {
        background: 'none', border: '1px solid #1e2a3a', borderRadius: 999,
        color: '#7d8ca0', padding: '2px 10px', cursor: 'pointer', fontSize: 12,
      },
    }

    function UserChip() {
      // undefined: not fetched yet (stay invisible — no flicker). null: the
      // session is gone. object: signed in.
      const [user, setUser] = React.useState(undefined)
      React.useEffect(() => {
        let alive = true
        const refresh = () => {
          fetch('/api/auth/me')
            .then(res => {
              if (!alive) return
              if (res.ok) {
                return res.json().then(body => { if (alive) setUser(body) })
              }
              // 401: the Alioth session is gone. It expires well before the
              // 30-day console cookie does, so a returning visitor can sit in
              // the console unauthenticated — render the signed-out chip
              // (login/logout must stay reachable) and drop the stale marker
              // cookie so the next index load lets the gate bounce to /landing.
              if (res.status === 401) {
                document.cookie = 'alioth_user=; Path=/; Max-Age=0'
                setUser(null)
              }
              // Any other failure (5xx, network): keep the last known identity.
            })
            .catch(() => {})
        }
        // Revalidate on tab focus/visibility: identity can change in another
        // tab (login/logout) — a mount-only fetch shows a stale badge.
        const onFocus = () => { refresh() }
        document.addEventListener('visibilitychange', onFocus)
        window.addEventListener('focus', onFocus)
        refresh()
        return () => {
          alive = false
          document.removeEventListener('visibilitychange', onFocus)
          window.removeEventListener('focus', onFocus)
        }
      }, [])
      if (user === undefined) return null
      if (user === null) {
        return e('div', { style: styles.chip },
          e('span', { style: styles.namespace }, '未登录'),
          e('a', { href: '/login', style: styles.link }, '登录'),
          e('a', { href: '/landing', style: styles.link }, '首页'))
      }
      const logout = () => {
        fetch('/api/auth/logout', { method: 'POST' })
          .catch(() => {})
          .then(() => {
            location.replace('/landing')
          })
      }
      return e('div', { style: styles.chip },
        e('span', { style: styles.name }, user.username),
        e('span', { style: styles.namespace }, user.namespace),
        // Workspace mode decides the entry: unlimited opens 工作区 (custom
        // workspace browser), standard is fixed to 应用 (the user's apps).
        e('a', { href: '/workspace', style: styles.link }, user.workspaceMode === 'unlimited' ? '工作区' : '应用'),
        e('a', { href: '/usercenter', style: styles.link }, '用户中心'),
        e('button', { style: styles.button, onClick: logout }, '退出'))
    }

    // ── Right-Sidebar tab: this app's artifacts + AppAgent state ───────────
    // The product's answer to the harness's generic right-Sidebar tabs: the Web
    // terminal is disabled in the bundle patch (a browser user must never hold a
    // shell as the service account), while this tab shows what the session is
    // actually working on. Data comes from the same-origin
    // GET /api/alioth/app-status?sessionId=…, which resolves the app from the
    // session's workspace and authorises the read against the caller's namespace.
    const ALIOTH_TAB_ID = '@dsh-alioth/sidebar-alioth'
    const ALIOTH_TAB_KIND = 'alioth'

    const panel = {
      root: { font: '13px/1.6 system-ui', color: '#d7e0ea', padding: '10px 12px', display: 'flex', flexDirection: 'column', gap: 10 },
      hint: { color: '#7d8ca0', padding: '10px 12px', font: '13px system-ui' },
      card: { border: '1px solid #1e2a3a', borderRadius: 10, padding: '8px 10px', background: '#0d1420' },
      cardTitle: { fontSize: 11, letterSpacing: '.08em', textTransform: 'uppercase', color: '#7d8ca0', marginBottom: 6 },
      row: { display: 'flex', justifyContent: 'space-between', gap: 10 },
      label: { color: '#7d8ca0', flex: '0 0 auto' },
      value: { textAlign: 'right', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
      mono: { fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace' },
      actions: { display: 'flex', gap: 6 },
      button: { background: 'none', border: '1px solid #1e2a3a', borderRadius: 999, color: '#7d8ca0', padding: '2px 10px', cursor: 'pointer', fontSize: 12 },
      ok: { color: '#3ee6a8' },
      warn: { color: '#e6b450' },
      bad: { color: '#ff7b72' },
      detail: { color: '#7d8ca0', fontSize: 12, marginTop: 4, wordBreak: 'break-word' },
      path: { color: '#5d6b7f', fontSize: 11, wordBreak: 'break-all' },
      fileLink: { color: '#4fc3f7', textDecoration: 'none', wordBreak: 'break-all' },
      assetLink: { color: '#7d8ca0', textDecoration: 'none', wordBreak: 'break-all' },
      dim: { color: '#5d6b7f', fontSize: 11, flex: '0 0 auto' },
      warnDetail: { color: '#e6b450', fontSize: 12, marginTop: 4, wordBreak: 'break-word' },
      okDetail: { color: '#3ee6a8', fontSize: 12, marginTop: 4, wordBreak: 'break-word' },
      badDetail: { color: '#ff7b72', fontSize: 12, marginTop: 4, wordBreak: 'break-word' },
    }

    /** One label/value line; `tone` colours the value when the state is notable. */
    function statusRow(label, value, tone) {
      return e('div', { style: panel.row },
        e('span', { style: panel.label }, label),
        e('span', { style: tone === undefined ? panel.value : Object.assign({}, panel.value, tone) }, value))
    }

    /** A titled group of rows (plus optional extra children). */
    function card(title, children) {
      return e('div', { style: panel.card },
        e('div', { style: panel.cardTitle }, title),
        ...children)
    }

    const EXTENSION_TONE = { passed: panel.ok, degraded: panel.warn, absent: panel.label }
    const EXTENSION_TEXT = { passed: '已装配', degraded: '降级（待人工门）', absent: '未验证' }

    /**
     * The tab body. Session-scoped slots receive `sessionId` from the framework;
     * everything else comes from our read-only status route.
     * @param props - framework props plus this type's injected `openFilesTab`.
     */
    function AliothAppBody(props) {
      const state = React.useState({ phase: 'loading' })
      const view = state[0]
      const setView = state[1]
      const sessionId = props.sessionId
      const load = React.useCallback(function () {
        if (!sessionId) { setView({ phase: 'nosession' }); return }
        setView({ phase: 'loading' })
        fetch('/api/alioth/app-status?sessionId=' + encodeURIComponent(sessionId))
          .then(function (res) {
            if (!res.ok) throw new Error('HTTP ' + res.status)
            return res.json()
          })
          .then(function (body) { setView({ phase: 'ready', body: body }) })
          .catch(function (err) { setView({ phase: 'error', error: String((err && err.message) || err) }) })
      }, [sessionId])
      React.useEffect(function () { load() }, [load])

      const refresh = e('button', { style: panel.button, onClick: load }, '刷新')
      const openPrototypes = e('button', { style: panel.button, onClick: props.openPrototypes }, '原型')

      if (view.phase === 'nosession') return e('div', { style: panel.hint }, '会话未就绪。')
      if (view.phase === 'loading') return e('div', { style: panel.hint }, '读取中…', e('div', { style: panel.actions }, refresh))
      if (view.phase === 'error') {
        return e('div', { style: panel.hint }, '读取失败：' + view.error, e('div', { style: panel.actions }, refresh))
      }
      const body = view.body
      if (body && body.app === null) {
        return e('div', { style: panel.hint }, '当前会话不在应用工作区内——在「新建会话」里选择一个应用后回到这里。')
      }

      const artifacts = body.artifacts
      const appJson = artifacts.appJson
      const pipeline = body.pipeline
      const run = pipeline.run
      const closure = pipeline.closure
      // 模型依赖显示：部署实际提供的模型 vs app.json 声明的下限（判定在服务端算好，客户端只画）。
      const model = body.model
      const dependency = body.dependency
      const modelText = model === null ? '未知（环境服务不可达）' : 'v' + model.version
      const dependencyText = dependency.declared === null || dependency.declared === ''
        ? '未声明'
        : '≥' + dependency.declared + (dependency.satisfied ? ' ✓ 满足' : ' ✗ 不满足')

      const appJsonTone = appJson.present && appJson.valid ? panel.ok : panel.bad
      const appJsonText = appJson.present
        ? (appJson.valid ? '契约通过' : appJson.errors.length + ' 项不合规')
        : '缺失'
      const runText = run.present === false
        ? '无 run 记录（尚未启动流水线）'
        : ('error' in run
            ? '读取失败'
            : '轨道 ' + run.trackIndex + ' · 步骤 ' + run.stepIndex + ' · 已完成 ' + run.completed + ' 步'
              + (run.lastCompleted ? '（最后 ' + run.lastCompleted + '）' : ''))

      const appCard = card('应用', [
        statusRow('工作区', body.app.code, panel.mono),
        statusRow('命名空间', body.app.namespace, panel.mono),
        e('div', { style: panel.path }, body.app.dir),
      ])
      const artifactCard = card('产物', [
        statusRow('app.json', appJsonText, appJsonTone),
        statusRow('名称 / 状态', (appJson.name || '—') + ' · ' + (appJson.status || '—')),
        statusRow('模型版本', modelText, model === null ? panel.warn : undefined),
        statusRow('模型依赖', dependencyText, dependency.satisfied ? panel.ok : (dependency.declared === null || dependency.declared === '' ? panel.label : panel.bad)),
        statusRow('模块 / 块', appJson.modules + ' / ' + appJson.blocks + '（磁盘 ' + artifacts.modulesOnDisk + ' 个 module.json）'),
        statusRow('extensions', artifacts.extensions.files + ' 个 yaml · ' + EXTENSION_TEXT[artifacts.extensions.verification], EXTENSION_TONE[artifacts.extensions.verification]),
        statusRow('Sources / 原型', artifacts.sources.dirs + ' 个目录 · prototype.html ' + (artifacts.prototype.html ? '有' : '无')),
        ...(appJson.errors.length === 0 ? [] : [e('div', { style: panel.detail }, '契约不合规：' + appJson.errors.slice(0, 3).join('；'))]),
      ])
      const pipelineChildren = [
        statusRow('流水线', runText, run.present === false ? panel.label : undefined),
        statusRow('未决门', pipeline.deferred.open + ' 项', pipeline.deferred.open > 0 ? panel.warn : panel.ok),
        statusRow('闭环裁决', closure.present ? closure.verdict + ' · #' + closure.seq + ' · ' + closure.at : '无记录',
          closure.present ? (closure.verdict === 'approved' ? panel.ok : panel.bad) : panel.label),
      ]
      const deferredDetails = pipeline.deferred.items.slice(0, 3).map(function (item) {
        return e('div', { style: panel.detail }, '· ' + ((item.app === null ? '未归属' : item.app) + '：' + item.reason))
      })
      return e('div', { style: panel.root },
        appCard,
        artifactCard,
        card('AppAgent', pipelineChildren.concat(deferredDetails)),
        e('div', { style: panel.actions }, refresh, openPrototypes))
    }

    /** The prototype tab: the ONLY file surface the console exposes. */
    const PROTOTYPE_TAB_ID = '@dsh-alioth/sidebar-prototype'
    const PROTOTYPE_TAB_KIND = 'alioth-prototype'

    /**
     * Prototype listing. Source is a paid, time-limited download, so this tab —
     * and the `/preview/…` allowlist behind every link — never shows it; the
     * harness's own file Remote is disabled in the bundle patch, not merely
     * unmounted.
     * @param props - framework props plus this type's injected `openStatusTab`.
     */
    function PrototypeBody(props) {
      const state = React.useState({ phase: 'loading' })
      const view = state[0]
      const setView = state[1]
      const sessionId = props.sessionId
      const load = React.useCallback(function () {
        if (!sessionId) { setView({ phase: 'nosession' }); return }
        setView({ phase: 'loading' })
        fetch('/api/alioth/prototypes?sessionId=' + encodeURIComponent(sessionId))
          .then(function (res) {
            if (!res.ok) throw new Error('HTTP ' + res.status)
            return res.json()
          })
          .then(function (body) { setView({ phase: 'ready', body: body }) })
          .catch(function (err) { setView({ phase: 'error', error: String((err && err.message) || err) }) })
      }, [sessionId])
      React.useEffect(function () { load() }, [load])

      const notice = React.useState(null)
      const setNotice = notice[1]
      const refresh = e('button', { style: panel.button, onClick: load }, '刷新')
      const status = e('button', { style: panel.button, onClick: props.openStatusTab }, '应用状态')
      // Source is paid: ask the server for a short-lived, account-bound link and
      // follow it. 402 carries the reason + where to subscribe, so the panel can
      // say what is missing instead of failing silently.
      const requestSource = function () {
        setNotice(null)
        fetch('/api/alioth/source/request', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ sessionId: sessionId }),
        }).then(function (res) {
          return res.json().then(function (body) { return { status: res.status, body: body } })
        }).then(function (result) {
          if (result.status === 200 && result.body && result.body.url) {
            setNotice({ kind: 'ok', text: '已签发限时链接（' + result.body.expiresAt + ' 前有效），开始下载…' })
            window.location.href = result.body.url
            return
          }
          if (result.status === 402) {
            // Source is L2 (¥4,999 起, 商务对接) — NOT the L1 subscription: the
            // message must send people to the tier that actually unlocks it.
            const until = result.body && result.body.until ? '（授权至 ' + result.body.until + '）' : ''
            setNotice({
              kind: 'need',
              text: (result.body && result.body.reason === 'expired'
                ? '源码下载授权已过期，续期后可下载'
                : '源码下载需 L2 授权（¥4,999 起，商务对接）') + until,
              url: (result.body && result.body.licenseUrl) || '/usercenter/subscription',
            })
            return
          }
          setNotice({ kind: 'error', text: '下载申请失败：' + ((result.body && result.body.error) || ('HTTP ' + result.status)) })
        }).catch(function (err) {
          setNotice({ kind: 'error', text: '下载申请失败：' + String((err && err.message) || err) })
        })
      }
      // Declared after `requestSource`: the button's onClick closes over it, and a
      // template literal would hit the TDZ if it were built first.
      const sourceButton = e('button', { style: panel.button, onClick: requestSource }, '下载源码')
      const noticeLine = notice[0] === null ? null : e('div', {
        style: notice[0].kind === 'need' ? panel.warnDetail : (notice[0].kind === 'ok' ? panel.okDetail : panel.badDetail),
      }, notice[0].text, notice[0].url === undefined ? null : e('a', { href: notice[0].url, style: panel.fileLink }, '查看 L2 授权'))

      if (view.phase === 'nosession') return e('div', { style: panel.hint }, '会话未就绪。')
      if (view.phase === 'loading') return e('div', { style: panel.hint }, '读取中…', e('div', { style: panel.actions }, refresh))
      if (view.phase === 'error') {
        return e('div', { style: panel.hint }, '读取失败：' + view.error, e('div', { style: panel.actions }, refresh))
      }
      const body = view.body
      if (body && body.app === null) {
        return e('div', { style: panel.hint }, '当前会话不在应用工作区内——在「新建会话」里选择一个应用后回到这里。')
      }
      const entries = (body && body.entries) || []
      if (entries.length === 0) {
        return e('div', { style: panel.root },
          e('div', { style: panel.hint }, '尚无原型产物：先让 Agent 生成原型（prototype.html），这里就会出现。'),
          e('div', { style: panel.actions }, sourceButton, refresh, status),
          noticeLine)
      }
      const section = function (title, group) {
        const rows = entries.filter(function (entry) { return entry.group === group })
        if (rows.length === 0) return null
        return card(title, rows.map(function (entry) {
          return e('div', { style: panel.row },
            e('a', {
              href: entry.url,
              target: '_blank',
              rel: 'noopener',
              style: entry.kind === 'html' ? panel.fileLink : panel.assetLink,
            }, entry.label),
            e('span', { style: panel.dim }, entry.kind === 'html' ? '' : '资源'))
        }))
      }
      return e('div', { style: panel.root },
        e('div', { style: panel.path }, body.app.namespace + ' / ' + body.app.code),
        section('应用原型', 'app'),
        section('命名空间原型（共享壳与资源）', 'namespace'),
        e('div', { style: panel.detail }, '源码不在控制台开放——L2 授权后可限时下载。'),
        e('div', { style: panel.actions }, sourceButton, refresh, status),
        noticeLine)
    }

    const prototypeTabDefinition = {
      id: PROTOTYPE_TAB_ID,
      kind: PROTOTYPE_TAB_KIND,
      priority: 'extension',
      title: function () { return '原型' },
      guide: [{
        id: 'alioth-prototypes',
        order: 20,
        title: function () { return '原型' },
        description: function () { return '本应用的原型页面与共享资源（源码不在控制台开放）' },
        icon: AliothTabGlyph,
      }],
    }

    // ── The operator's console surfaces: loopback only ────────────────────
    // Two harness surfaces belong to the person at the machine, not to a
    // multi-tenant browser:
    //   * 设置 (模型 / 内置插件 / Agent 预设) reads and writes the Host's own
    //     configuration, and its rich actions are themselves gated on
    //     `connection.isLoopback` — off loopback it degrades into a
    //     half-broken page (the provider directory fails to load).
    //   * 插件 installs, enables, disables and removes the Host's plugin rows
    //     through the `pluginManager` Remote — a serving-host mutation.
    // This console is reached through domains, reverse proxies and tunnels, so
    // both exist for the operator at localhost / 127.0.0.1 and nowhere else.
    // The predicate is the harness's own loopback rule — the one behind the
    // /api Host fence and `connection.isLoopback` (localhost, IPv6 loopback, or
    // any 127/8 literal).
    const SETTINGS_SEAT = 'sidebar.settings'
    /** `ui-plugin-manager`'s `PANEL_ID`: the sidebar entry and the page it opens. */
    const PLUGIN_PANEL_ID = 'plugins'
    const REGISTRANT = '@dsh-alioth/auth-web-alioth'

    /** Loopback page authority — mirrors the harness's `isLoopbackHostname`. */
    function isLoopbackAuthority(hostname) {
      if (hostname === 'localhost' || hostname === '[::1]') return true
      const parts = String(hostname || '').split('.')
      return parts.length === 4
        && parts[0] === '127'
        && parts.every(function (part) { return /^\d{1,3}$/.test(part) && Number(part) <= 255 })
    }

    /**
     * Whether the operator's surfaces belong on this page. `connection.isLoopback`
     * is the harness's own answer (a shell that owns its Host reports true);
     * the page authority is the fallback while that service is not yet mounted
     * at apply time. No authority at all means no loopback evidence — the
     * surfaces stay hidden (fail-closed: an unprovable authority is not the
     * operator's machine).
     * @param ctx - client root context (cordis ClientContext).
     */
    function operatorSurfacesVisible(ctx) {
      const connection = ctx && typeof ctx.get === 'function' ? ctx.get('connection') : undefined
      if (connection && typeof connection.isLoopback === 'boolean') return connection.isLoopback
      const location = globalThis.location
      return isLoopbackAuthority(location && location.hostname)
    }

    /** Empty occupant: shadowing the shipped settings shell renders nothing. */
    function NoSettingsSeat() { return null }

    /**
     * Off loopback the 插件 (plugins) panel row is shadowed by this occupant.
     * The sidebar draws a panel row's chrome (button, label, tooltip) from the
     * ledger metadata and only the glyph from the `sidebar.panellist` cell, so
     * a cell cannot drop its row by rendering nothing: the occupant hides the
     * row it is rendered into instead. Fail-open by construction — if the row
     * is no longer an ancestor button, the row simply stays visible (the
     * browser E2E pins the hidden state).
     */
    function HiddenPluginPanel() {
      const ref = React.useRef(null)
      React.useEffect(function () {
        const node = ref.current
        const row = node && typeof node.closest === 'function' ? node.closest('button') : null
        if (row) row.style.display = 'none'
      })
      return e('span', { ref: ref, style: { display: 'none' } })
    }

    /** The panel page behind that row: unreachable even from a stale selection. */
    function NoPanelPage() { return null }

    /** The tab type's guide glyph (the guide capsule renders `icon` at its size). */
    function AliothTabGlyph(props) {
      const size = props && props.size ? props.size : 16
      return e('span', { style: { fontSize: size + 'px', lineHeight: 1, color: '#3ee6a8' } }, '◈')
    }

    /**
     * Stage one: what the `alioth` tab type IS. No resource patterns — it is a
     * page whose content is this session's app, so users reach it from the guide.
     */
    const aliothTabDefinition = {
      id: ALIOTH_TAB_ID,
      kind: ALIOTH_TAB_KIND,
      priority: 'extension',
      title: function () { return '应用状态' },
      guide: [{
        id: 'alioth-app',
        order: 30,
        title: function () { return '应用状态' },
        description: function () { return '当前应用的产物契约、扩展装配验证与 AppAgent 流水线状态' },
        icon: AliothTabGlyph,
      }],
    }

    /**
     * Client plugin body: one additive entry in the frame overlay layer.
     * Registration defers through ctx.slots.inject — shell.overlay is declared
     * by ui-layout's AppFrame, and direct register() before that declaration
     * throws "slot is not declared" (plugin activation order is not ours to
     * control).
     * @param ctx - client root context (cordis ClientContext).
     */
    function apply(ctx) {
      ctx.effect(() => ctx.slots.inject('shell.overlay', () =>
        ctx.slots.register({ name: 'shell.overlay', id: 'alioth-user-chip' }, UserChip)))
      // Off loopback the operator surfaces are shadowed at priority -1: a
      // single slot renders its lowest live entry, and a list/keyed cell
      // renders the first live entry per id/key in priority order — our
      // occupant wins either way while the shipped registrants keep owning
      // their child declarations, so registrants waiting on `settings.*`
      // seats are left undisturbed. On loopback nothing is registered and the
      // harness surfaces stand as shipped.
      if (!operatorSurfacesVisible(ctx)) {
        ctx.effect(() => ctx.slots.inject(SETTINGS_SEAT, () => ctx.slots.register({
          name: SETTINGS_SEAT,
          priority: -1,
          registrant: REGISTRANT,
        }, NoSettingsSeat)))
        ctx.effect(() => ctx.slots.inject('sidebar.panellist', () => ctx.slots.register({
          name: 'sidebar.panellist',
          id: PLUGIN_PANEL_ID,
          priority: -1,
          registrant: REGISTRANT,
        }, HiddenPluginPanel)))
        ctx.effect(() => ctx.slots.inject('main', () => ctx.slots.register({
          name: 'main',
          key: PLUGIN_PANEL_ID,
          priority: -1,
          registrant: REGISTRANT,
        }, NoPanelPage)))
      }
      // The right-Sidebar tab registers only where that registry exists (web
      // profiles); a tree without it keeps the chip and gains nothing else.
      ctx.inject(['sidebarRightTabs'], (scope) => {
        scope.effect(() => scope.sidebarRightTabs.register(aliothTabDefinition))
        scope.effect(() => scope.slots.inject('sidebar.right.pane.tab', () => scope.slots.register({
          name: 'sidebar.right.pane.tab',
          key: ALIOTH_TAB_ID,
          inject: () => ({
            openPrototypes: () => scope.sidebarRight.openTab(PROTOTYPE_TAB_KIND),
          }),
        }, AliothAppBody)))
        scope.effect(() => scope.sidebarRightTabs.register(prototypeTabDefinition))
        scope.effect(() => scope.slots.inject('sidebar.right.pane.tab', () => scope.slots.register({
          name: 'sidebar.right.pane.tab',
          key: PROTOTYPE_TAB_ID,
          inject: () => ({
            openStatusTab: () => scope.sidebarRight.openTab(ALIOTH_TAB_KIND),
          }),
        }, PrototypeBody)))
      })
    }

    exports.inject = ['slots']
    exports.apply = apply
    return exports
  },
})
