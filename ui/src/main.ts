import "./clientInit"; // must be first: sets SDK baseUrl before any composable runs
import { createApp } from "vue";
import { router } from "./router";
import App from "./App.vue";
import "./assets/index.css";

createApp(App).use(router).mount("#app");
