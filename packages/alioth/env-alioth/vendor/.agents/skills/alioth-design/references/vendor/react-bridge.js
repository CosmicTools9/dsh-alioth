/* react-bridge.js — React 19 react-umd compatibility bridge for browser UMD
 *
 * React 19 react-umd package splits the rendering API into two modules:
 *   react-dom.umd.js       → window.ReactDOM (no createRoot)
 *   react-dom-client.umd.js → window.ReactDOMClient (has createRoot/hydrateRoot)
 *
 * This bridge re-exports ReactDOMClient methods onto ReactDOM and provides
 * a backward-compatible ReactDOM.render() polyfill so all prototype code
 * (both React 18 and React 19 patterns) works unchanged.
 *
 * Load after the three vendor files in this order:
 *   <script src="../react.umd.js"></script>
 *   <script src="../react-dom.umd.js"></script>
 *   <script src="../react-dom-client.umd.js"></script>
 *   <script src="../react-bridge.js"></script>
 */
(function(){
  var CLIENT = window.ReactDOMClient;
  var DOM = window.ReactDOM;
  if (!CLIENT || !DOM) return;

  // Forward createRoot/hydrateRoot from ReactDOMClient onto ReactDOM
  DOM.createRoot = CLIENT.createRoot;
  DOM.hydrateRoot = CLIENT.hydrateRoot;

  // Backward-compatible ReactDOM.render() polyfill (React 18 pattern)
  DOM.render = function(element, container, callback) {
    var root = CLIENT.createRoot(container);
    root.render(element);
    if (typeof callback === 'function') callback.call(element);
    return root;
  };

  // Backward-compatible ReactDOM.hydrate() polyfill
  DOM.hydrate = function(element, container, callback) {
    var root = CLIENT.hydrateRoot(container, element);
    if (typeof callback === 'function') callback.call(element);
    return root;
  };

  // Backward-compatible unmountComponentAtNode
  DOM.unmountComponentAtNode = function(container) {
    try { container._reactRootContainer = null; } catch(e) {}
  };
})();
