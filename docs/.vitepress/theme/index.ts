import DefaultTheme from 'vitepress/theme'
// Declare, map, use — in that order. `tokens.css` is copied from
// `ui/src/design/tokens.css` by `task ui:tokens` and is the single place a
// colour, step or duration is decided; `vp-bridge.css` maps those tokens onto
// VitePress's `--vp-*` contract; `custom.css` is component rules that spend
// them. A component stylesheet loading before the tokens it references is the
// failure this ordering exists to prevent (RFC 0005 §5.2).
import './tokens.css'
import './vp-bridge.css'
import './custom.css'
import ConfigGenerator from '../components/ConfigGenerator.vue'
import Layout from './Layout.vue'
import type { Theme } from 'vitepress'

/**
 * A code block that scrolls has to be reachable by keyboard (WCAG 2.1.1) —
 * otherwise the only way to read the right-hand end of a long `docker run` line
 * is a mouse. The default theme gives `pre` `overflow-x: auto` and no
 * `tabindex`, which `docs:design:rendered` reports as
 * `axe:scrollable-region-focusable`. It cannot be fixed in CSS, and replacing
 * the whole code-block component to add one attribute would be a far larger
 * surface to maintain than this.
 */
function makeCodeBlocksFocusable() {
  for (const pre of document.querySelectorAll<HTMLElement>('.vp-doc pre')) {
    if (!pre.hasAttribute('tabindex')) pre.setAttribute('tabindex', '0')
  }
}

export default {
  extends: DefaultTheme,
  Layout,
  enhanceApp({ app, router }) {
    app.component('ConfigGenerator', ConfigGenerator)
    if (typeof window !== 'undefined') {
      // Once for the server-rendered page, then after every client navigation.
      // `requestAnimationFrame` waits for the new page's DOM rather than the
      // outgoing one's.
      const run = () => requestAnimationFrame(makeCodeBlocksFocusable)
      router.onAfterRouteChange = run
      run()
    }
  },
} satisfies Theme
