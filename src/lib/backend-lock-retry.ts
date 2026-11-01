// O1.2（P0）：空闲自动锁的后端密钥清除——带重试与显式失败上报。
//
// 背景（O1.1 审查结论）：空闲自动锁此前只调用 appStore.lock()（纯前端 pinia 状态），
// 后端 ActiveKey 原样保留——锁屏显示期间 list_passwords 等受保护命令实测仍可返回数据。
//
// 契约：
// - 调用方必须先同步切前端锁屏（立即遮蔽敏感内容），再调用本函数清后端密钥；
// - lockFn 即既有后端 lock_app 命令（= clear_key，幂等），不新建锁系统；
// - lock_app 仅在 IPC 通道失效时才会失败，而通道失效通常意味着后端进程正在退出
//   （密钥随进程消亡）；但若后端进程仍存活则密钥未清——必须显式上报，不得静默当作已锁。
export interface BackendLockRetryOptions {
  /** 总尝试次数（含首次），默认 3 */
  attempts?: number;
  /** 重试间隔（毫秒），默认 500 */
  retryDelayMs?: number;
  /** 每次失败的回调（日志/上报用）；attempt 从 1 开始 */
  onError?: (error: unknown, attempt: number) => void;
  /** 等待函数（测试注入用，替换真实 setTimeout 等待） */
  delay?: (ms: number) => Promise<void>;
}

/**
 * 依次尝试 lockFn（后端 lock_app），失败重试直至用尽次数。
 * 返回 true=后端密钥已清除；false=全部尝试失败（调用方必须显式上报，不得静默）。
 * lockFn 的同步异常与 Promise 拒绝一律按失败处理，不会向上抛出。
 */
export async function clearBackendKeyWithRetry(
  lockFn: () => Promise<void>,
  options: BackendLockRetryOptions = {},
): Promise<boolean> {
  const attempts = Math.max(1, options.attempts ?? 3);
  const retryDelayMs = options.retryDelayMs ?? 500;
  const delay = options.delay ?? ((ms: number) => new Promise((r) => setTimeout(r, ms)));
  for (let attempt = 1; attempt <= attempts; attempt++) {
    try {
      await lockFn();
      return true;
    } catch (e) {
      options.onError?.(e, attempt);
      if (attempt < attempts) await delay(retryDelayMs);
    }
  }
  return false;
}
