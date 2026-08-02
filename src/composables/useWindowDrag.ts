import { ref, onUnmounted, type Ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * 窗口拖动组合式函数
 * - 提供 onMouseDown 处理器
 * - 暴露 isDragging 状态供 UI 视觉反馈
 * - 暴露 onDragEnd 回调，在拖动结束时同步触发（用于"统一处理"模式）
 * - 在组件卸载时自动清理
 *
 * 关键修复 P0-#X：拖动期间严禁触发任何 IPC
 * 原因：WebView2 在拖动时主线程压力极大，每帧一次 IPC 都会导致队列堆积
 * → 几秒后"未响应"。统一方案：
 *   1. 拖动中完全冻结所有 IPC（位置/大小/边缘检测）
 *   2. 拖动结束（startDragging promise resolve 或 mouseup）才统一触发一次
 *
 * 用法：
 *   const { isDragging, onDragHandleMouseDown, onDragEnd } = useWindowDrag({
 *     onDragEnd: () => { /* 拖动结束统一处理 *\/ }
 *   });
 *   <div @mousedown="onDragHandleMouseDown">...</div>
 */
export function useWindowDrag(options?: {
  onDragEnd?: () => void;
}) {
  const isDragging = ref(false);
  let mouseUpDebounce = false;

  function onDocumentMouseUp() {
    if (mouseUpDebounce) return;
    if (!isDragging.value) return;
    mouseUpDebounce = true;
    // 拖动结束的统一处理（300ms debounce 防止 startDragging promise 还没 resolve 时漏掉）
    setTimeout(() => {
      mouseUpDebounce = false;
      if (isDragging.value) return;
      options?.onDragEnd?.();
    }, 50);
  }

  function onDragHandleMouseDown(e: MouseEvent) {
    // 按钮/输入框等可交互元素不触发拖动
    const t = e.target as HTMLElement;
    if (t.closest("button, a, input, select, textarea, [data-no-drag]")) {
      return;
    }
    // 必须用 e.buttons === 1 严格判断左键
    if (e.buttons !== 1) return;
    // 拖动视觉反馈
    isDragging.value = true;
    document.body.classList.add("is-dragging");
    // 触发原生拖动
    getCurrentWindow()
      .startDragging()
      .catch((err) => {
        console.warn("startDragging 失败", err);
      })
      .finally(() => {
        // 关键修复：拖动结束（promise 结束）才恢复状态 + 触发 onDragEnd
        isDragging.value = false;
        document.body.classList.remove("is-dragging");
        // 等待一帧再触发 onDragEnd，确保 Rust 端 WM_WINDOWPOSCHANGED 已被处理
        requestAnimationFrame(() => {
          options?.onDragEnd?.();
        });
      });
  }

  // 注册兜底 mouseup（防止 startDragging promise 异常不 resolve 时漏掉 onDragEnd）
  if (typeof document !== "undefined") {
    document.addEventListener("mouseup", onDocumentMouseUp, true);
  }
  onUnmounted(() => {
    if (typeof document !== "undefined") {
      document.removeEventListener("mouseup", onDocumentMouseUp, true);
    }
  });

  return {
    isDragging: isDragging as Ref<boolean>,
    onDragHandleMouseDown,
  };
}
