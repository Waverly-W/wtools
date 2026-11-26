import { mount } from "svelte";
import "../styles/base.css";
import "../styles/components.css";

// 开发模式下在普通浏览器打开时，先加载 Tauri 模拟层（生产构建会剔除）。
// App 必须在模拟层之后动态导入，因为部分模块在导入时就会访问 Tauri 接口。
if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
  await import("../dev/tauriMock");
}
const { default: App } = await import("./App.svelte");

mount(App, { target: document.getElementById("app")! });
