// O2：启动期非关键扫描的延迟执行器（首屏与用户交互优先）。
//
// 背景：MainLayout 启动时 health_check_apps（挂载后 +200ms）与 rescan_subtypes
// （+400ms）为全表扫描，与首屏视图加载并发竞争 data gate。改造后两者经本执行器
// 延迟到浏览器空闲时执行。
//
// 语义：
// - minDelayMs（默认 300）：首屏优先——启动后至少延迟该时长再排期；
// - requestIdleCallback 优先（浏览器空闲时执行；用户交互优先于空闲回调）；
// - setTimeout(timeoutMs) 兜底（默认 3000）：浏览器 idle 实现缺陷 / 窗口隐藏
//   时 idle 回调可能不触发，超时强制执行——**保证检查最终仍然执行，不以跳过冒充优化**；
// - started 幂等守卫：重复调度不重复执行，防并发重入；
// - cancel() 清理挂起中的排期（组件卸载时调用；已开始执行的任务不受 cancel 影响——
//   扫描命令本身幂等，且应用退出时进程随之消亡）。
export interface DeferredTaskOptions {
  /** 最小延迟（ms，默认 300）——首屏渲染优先 */
  minDelayMs?: number;
  /** 空闲回调超时兜底（ms，默认 3000）——保证最终执行 */
  timeoutMs?: number;
  /** 任务失败回调（日志用） */
  onError?: (e: unknown) => void;
}

type IdleCb = (cb: () => void, opts?: { timeout: number }) => number;
type IdleCancel = (id: number) => void;

const w = (typeof window !== "undefined"
  ? (window as unknown as { requestIdleCallback?: IdleCb; cancelIdleCallback?: IdleCancel })
  : ({} as { requestIdleCallback?: IdleCb; cancelIdleCallback?: IdleCancel }));

export function scheduleDeferred(
  run: () => Promise<void> | void,
  options: DeferredTaskOptions = {},
): { cancel: () => void; isStarted: () => boolean } {
  const minDelayMs = options.minDelayMs ?? 300;
  const timeoutMs = options.timeoutMs ?? 3000;
  let started = false;
  let minTimer: ReturnType<typeof setTimeout> | null = null;
  let idleTimer: ReturnType<typeof setTimeout> | null = null;
  let idleId: number | null = null;

  const execute = () => {
    if (started) return;
    started = true;
    try {
      Promise.resolve(run()).then(
        () => undefined,
        (e) => {
          options.onError?.(e);
          console.warn("[defer-idle] 任务失败", e);
        },
      );
    } catch (e) {
      // run() 同步抛出：同样按失败处理，不向外传播
      options.onError?.(e);
      console.warn("[defer-idle] 任务异常", e);
    }
  };

  const scheduleAt = () => {
    if (started) return;
    // 空闲回调优先 + setTimeout 双重兜底（谁先到谁执行，started 防重）
    if (typeof w.requestIdleCallback === "function") {
      idleId = w.requestIdleCallback(execute, { timeout: timeoutMs });
    }
    idleTimer = setTimeout(execute, timeoutMs);
  };

  minTimer = setTimeout(scheduleAt, minDelayMs);

  return {
    cancel: () => {
      if (minTimer) clearTimeout(minTimer);
      if (idleTimer) clearTimeout(idleTimer);
      if (idleId != null && w.cancelIdleCallback) w.cancelIdleCallback(idleId);
    },
    isStarted: () => started,
  };
}
