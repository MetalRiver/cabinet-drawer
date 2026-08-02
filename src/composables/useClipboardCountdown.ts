// 剪贴板倒计时统一管理
// 修复 P1-#19：之前 PasswordView 用 15s、SnippetsStore 用 30s，但都写同一个
// appStore.clipboardCountdown 且各自维护 setInterval；切换页面会互相"清零"。
// 改成全局单一 composable，每次新复制覆盖倒计时，旧 interval 自动清理。

import { useAppStore } from "../stores/app";

let activeTimer: number | null = null;

export function useClipboardCountdown() {
  const appStore = useAppStore();

  // 组件卸载时不要清零（其他组件可能还在用），只清掉自己创的 timer
  // 但因为我们这里 timer 是全局的，只能清零一次
  function clear() {
    if (activeTimer != null) {
      clearInterval(activeTimer);
      activeTimer = null;
      appStore.clipboardCountdown = 0;
    }
  }

  /**
   * 启动倒计时（每次调用会重置）
   * @param seconds 总秒数
   */
  function start(seconds: number) {
    // 修复 P1-#19：清掉旧 timer，重置倒计时
    if (activeTimer != null) {
      clearInterval(activeTimer);
      activeTimer = null;
    }
    appStore.clipboardCountdown = seconds;
    activeTimer = window.setInterval(() => {
      if (appStore.clipboardCountdown > 0) {
        appStore.clipboardCountdown--;
      } else {
        if (activeTimer != null) {
          clearInterval(activeTimer);
          activeTimer = null;
        }
      }
    }, 1000);
  }

  // 组件卸载兜底：仅当倒计时已归零才不清理，否则一直留着 timer 给其他组件用
  // 不在 onUnmounted 里清 timer，避免切页时清掉别的组件的倒计时

  return { start, clear, get secondsLeft() { return appStore.clipboardCountdown; } };
}
