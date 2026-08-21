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
    '.afb-hint{position:fixed;left:12px;bottom:12px;z-index:2147483646;background:#101724;color:#d7e0ea;border:1px solid #1e2a3a;border-radius:8px;padding:8px 12px;font:12px system-ui;box-shadow:0 2px 10px rgba(0,0,0,.4)}',
    '.afb-box{position:fixed;z-index:2147483647;background:#101724;color:#d7e0ea;border:1px solid #3ee6a8;border-radius:10px;padding:12px;font:13px system-ui;box-shadow:0 4px 20px rgba(0,0,0,.5);width:280px}',
    '.afb-box h4{margin:0 0 8px;font-size:12px;color:#3ee6a8;font-family:ui-monospace,monospace}',
    '.afb-box textarea{width:100%;height:64px;background:#070b11;color:#d7e0ea;border:1px solid #1e2a3a;border-radius:6px;padding:6px;font:12px system-ui;box-sizing:border-box}',
    '.afb-box .afb-row{margin-top:8px;text-align:right}',
    '.afb-box button{background:#3ee6a8;border:none;border-radius:6px;color:#06251a;font-weight:600;padding:5px 14px;font-size:12px;cursor:pointer;margin-left:6px}',
    '.afb-box button.ghost{background:none;border:1px solid #1e2a3a;color:#7d8ca0}',
    '.afb-pin{position:absolute;z-index:2147483645;background:#3ee6a8;color:#06251a;border-radius:999px;width:20px;height:20px;font:bold 11px system-ui;display:flex;align-items:center;justify-content:center;cursor:help;box-shadow:0 1px 6px rgba(0,0,0,.5)}',
    '.afb-toast{position:fixed;top:12px;left:50%;transform:translateX(-50%);z-index:2147483647;background:#122019;color:#3ee6a8;border:1px solid #3ee6a8;border-radius:999px;padding:6px 16px;font:12px system-ui}'
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
