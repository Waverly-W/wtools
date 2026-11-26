import { mount } from "svelte";
import "../styles/base.css";
import "../styles/components.css";
import "./json.css";

if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  await import("../dev/tauriMock");
}
const { default: App } = await import("./App.svelte");
mount(App, { target: document.getElementById("app")! });
