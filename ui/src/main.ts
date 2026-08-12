import "./clientInit"; // must be first: sets SDK baseUrl before any composable runs
import { createApp } from "vue";
import { router } from "./router";
import { i18n, initLocale } from "./i18n";
import App from "./App.vue";
import "./assets/index.css";

// `lang` on <html> was hardcoded to "en"; set it from the resolved locale before
// mount, so assistive tech and browser translation prompts are told the truth.
initLocale();

createApp(App).use(router).use(i18n).mount("#app");
