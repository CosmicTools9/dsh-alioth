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
 * What it does: registers the logged-in user chip into the frame-wide
 * `shell.overlay` list slot (the documented additive seat for status pills).
 * Identity comes from the same-origin `/api/auth/me` (HttpOnly session
 * cookie); logout posts `/api/auth/logout` (clears cookies server-side) and
 * bounces to /landing.
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
      button: {
        background: 'none', border: '1px solid #1e2a3a', borderRadius: 999,
        color: '#7d8ca0', padding: '2px 10px', cursor: 'pointer', fontSize: 12,
      },
    }

    function UserChip() {
      const [user, setUser] = React.useState(null)
      React.useEffect(() => {
        let alive = true
        const refresh = () => {
          fetch('/api/auth/me')
            .then(res => (res.ok ? res.json() : null))
            .then(body => { if (alive) setUser(body) })
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
      if (user === null) return null
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
        // Deployment environment decides the entry: local exposes 工作区
        // (workspace browser), production is fixed to 应用 (the user's apps).
        e('a', {
          href: '/workspace',
          style: { color: '#4fc3f7', fontSize: 12, textDecoration: 'none', padding: '0 2px' },
        }, user.environment === 'production' ? '应用' : '工作区'),
        e('a', {
          href: '/usercenter',
          style: { color: '#4fc3f7', fontSize: 12, textDecoration: 'none', padding: '0 2px' },
        }, '用户中心'),
        e('button', { style: styles.button, onClick: logout }, '退出'))
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
    }

    exports.inject = ['slots']
    exports.apply = apply
    return exports
  },
})
