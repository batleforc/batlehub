import DefaultTheme from 'vitepress/theme'
import './custom.css'
import ConfigGenerator from '../components/ConfigGenerator.vue'
import type { Theme } from 'vitepress'

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component('ConfigGenerator', ConfigGenerator)
  },
} satisfies Theme
