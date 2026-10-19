/**
 * The on-page annotation overlay (vanilla JS, zero dependencies), served at
 * /feedback/overlay.js. Inject via the bookmarklet on the /feedback page:
 * Alt+Click any element → leave a comment → POST {comment, url, element,
 * elementPath, cssClasses} to the feedback server. Rendered annotations of
 * the current page float as numbered pins.
 */
export const OVERLAY_JS = `(function () {
  if (window.__aliothFeedback) return
  var BASE = document.currentScript && document.currentScript.src ? document.currentScript.src.replace(/\\/feedback\\/overlay.js.*$/, '') : (window.__ALIOTH_FEEDBACK_BASE__ || '')
  if (!BASE) { console.error('[alioth-feedback] base URL unknown'); return }
  window.__aliothFeedback = true

  var style = document.createElement('style')
  style.textContent = [
    // Palette mirrors the DeepSeek Harness design tokens; the overlay floats
    // over host pages, so it carries its own light/dark aliases.
    '.afb-hint,.afb-box,.afb-pin,.afb-toast{--afb-bg:#fff;--afb-text:#0f1115;--afb-line:rgba(0,0,0,.10);--afb-input:#f9fafb;--afb-accent:#0f1115;--afb-accent-fg:#fff;--afb-ok:#22c55e}',
    '@media(prefers-color-scheme:dark){.afb-hint,.afb-box,.afb-pin,.afb-toast{--afb-bg:#232324;--afb-text:#f9fafb;--afb-line:rgba(255,255,255,.12);--afb-input:#1b1b1c;--afb-accent:#f9fafb;--afb-accent-fg:#0f1115}}',
    '.afb-hint{position:fixed;left:12px;bottom:12px;z-index:2147483646;background:var(--afb-bg);color:var(--afb-text);border:1px solid var(--afb-line);border-radius:8px;padding:8px 12px;font:12px system-ui;box-shadow:0 2px 10px rgba(0,0,0,.4)}',
    '.afb-box{position:fixed;z-index:2147483647;background:var(--afb-bg);color:var(--afb-text);border:1px solid var(--afb-line);border-radius:10px;padding:12px;font:13px system-ui;box-shadow:0 4px 20px rgba(0,0,0,.5);width:280px}',
    '.afb-box h4{margin:0 0 8px;font-size:12px;color:var(--afb-text);font-family:ui-monospace,monospace}',
    '.afb-box textarea{width:100%;height:64px;background:var(--afb-input);color:var(--afb-text);border:1px solid var(--afb-line);border-radius:6px;padding:6px;font:12px system-ui;box-sizing:border-box}',
    '.afb-box .afb-row{margin-top:8px;text-align:right}',
    '.afb-box button{background:var(--afb-accent);border:none;border-radius:6px;color:var(--afb-accent-fg);font-weight:600;padding:5px 14px;font-size:12px;cursor:pointer;margin-left:6px}',
    '.afb-box button.ghost{background:none;border:1px solid var(--afb-line);color:var(--afb-text)}',
    '.afb-pin{position:absolute;z-index:2147483645;background:var(--afb-accent);color:var(--afb-accent-fg);border-radius:999px;width:20px;height:20px;font:bold 11px system-ui;display:flex;align-items:center;justify-content:center;cursor:help;box-shadow:0 1px 6px rgba(0,0,0,.5)}',
    '.afb-toast{position:fixed;top:12px;left:50%;transform:translateX(-50%);z-index:2147483647;background:var(--afb-bg);color:var(--afb-text);border:1px solid var(--afb-ok);border-radius:999px;padding:6px 16px;font:12px system-ui}'
  ].join('')
  document.head.appendChild(style)

  var hint = document.createElement('div')
  hint.className = 'afb-hint'
  hint.textContent = '批注模式：Alt+点击元素'
  document.body.appendChild(hint)
  setTimeout(function () { hint.remove() }, 8000)

  function cssPath(el) {
    if (!(el instanceof Element)) return ''
    var parts = []
    var node = el
    while (node && node.nodeType === 1 && parts.length < 12) {
      var part = node.tagName.toLowerCase()
      if (node.id) { part += '#' + node.id; parts.unshift(part); break }
      if (node.className && typeof node.className === 'string') {
        var cls = node.className.trim().split(/\\s+/).slice(0, 2).join('.')
        if (cls) part += '.' + cls
      }
      parts.unshift(part)
      node = node.parentElement
    }
    return parts.join(' > ')
  }

  function toast(text) {
    var t = document.createElement('div')
    t.className = 'afb-toast'
    t.textContent = text
    document.body.appendChild(t)
    setTimeout(function () { t.remove() }, 3000)
  }

  document.addEventListener('click', function (event) {
    if (!event.altKey) return
    event.preventDefault()
    event.stopPropagation()
    var target = event.target
    var old = document.querySelector('.afb-box')
    if (old) old.remove()

    var box = document.createElement('div')
    box.className = 'afb-box'
    var title = document.createElement('h4')
    title.textContent = '批注 ' + cssPath(target).slice(0, 60)
    var area = document.createElement('textarea')
    area.placeholder = '问题描述（如：按钮错位、文案不对）'
    var row = document.createElement('div')
    row.className = 'afb-row'
    var cancel = document.createElement('button')
    cancel.className = 'ghost'
    cancel.textContent = '取消'
    var submit = document.createElement('button')
    submit.textContent = '提交批注'
    row.appendChild(cancel)
    row.appendChild(submit)
    box.appendChild(title)
    box.appendChild(area)
    box.appendChild(row)
    box.style.left = Math.min(window.innerWidth - 300, event.clientX + 12) + 'px'
    box.style.top = Math.min(window.innerHeight - 180, event.clientY + 12) + 'px'
    document.body.appendChild(box)
    area.focus()

    cancel.onclick = function () { box.remove() }
    submit.onclick = function () {
      var comment = area.value.trim()
      if (!comment) { area.focus(); return }
      submit.disabled = true
      fetch(BASE + '/api/feedback/annotations', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          origin: location.origin,
          url: location.href,
          comment: comment,
          element: target.tagName ? target.tagName.toLowerCase() : String(target.nodeName),
          elementPath: cssPath(target),
          cssClasses: typeof target.className === 'string' ? target.className : ''
        })
      }).then(function (res) {
        box.remove()
        if (res.ok) toast('批注已提交 #' + (res.status))
        else toast('提交失败：' + res.status)
      }).catch(function (err) { box.remove(); toast('提交失败：' + err) })
    }
  }, true)
})()
`
