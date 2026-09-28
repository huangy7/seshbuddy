import { ref, shallowRef, triggerRef, onUnmounted, getCurrentInstance, type Ref, type ShallowRef } from 'vue'
import { invokeApp, renderAppError, wrapAppError, type AppErrorPayload } from '../utils/invokeApp'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export interface StartOptions {
  /**
   * 原子替换模式（SWR 刷新）。
   * 为 true 时，不立即将 `items` 重置为空数组，
   * 而是将新收到的 chunks 暂存在内部缓冲区中，待 done 事件触发时一次性原子赋值给 `items`。
   * 从而保证在重新加载期间旧数据始终在界面展示，绝无白屏/骨架屏二次闪烁。
   */
  atomicSwap?: boolean
}

export interface UseStreamingLoadReturn<TChunk, TDone> {
  /** 累积的数据项（chunk 是 batch，items 是 flat 数组） */
  items: ShallowRef<TChunk[]>
  /** done 事件的 payload；未完成时为 null */
  done: Ref<TDone | null>
  /** 错误对象；正常时为 null */
  error: Ref<Error | null>
  /** 正在加载中（start 调用后直到收到 done 或 error） */
  loading: Ref<boolean>
  /**
   * 启动一次流式加载。会自动清理上一次的 listener。
   * - `commandSuffix`: 后端命令名（不含 `_stream`，例如 `'load_session'`）
   * - `topic`: 事件 topic（与后端约定，如 `'session'`）
   * - `args`: 透传给后端命令的参数；`requestId` 由 composable 自动生成
   * - `options`: 启动选项，例如 `{ atomicSwap: true }`
   */
  start: (
    commandSuffix: string,
    topic: string,
    args: Record<string, unknown>,
    options?: StartOptions
  ) => Promise<void>
  /** 主动放弃当前流（清理监听 + reset 状态）。不会通知后端，后端继续跑完丢弃 emit */
  cancel: () => void
  /** 等待当前流完全加载完毕（done 或 error）。如果当前未在加载，立即 resolve；支持超时兜底避免永久悬挂 */
  waitForCompletion: (timeoutMs?: number) => Promise<TDone | null>
}

/**
 * 必须在组件 `setup()` 同步阶段调用，依赖 `onUnmounted` 自动清理监听器。
 * 在非 setup 环境（store/外层模块）下使用时需自行调用 cancel()。
 */
export function useStreamingLoad<TChunk, TDone>(): UseStreamingLoadReturn<TChunk, TDone> {
  const items = shallowRef<TChunk[]>([])
  const done = ref<TDone | null>(null) as Ref<TDone | null>
  const error = ref<Error | null>(null)
  const loading = ref(false)

  let unlistenChunk: UnlistenFn | null = null
  let unlistenDone: UnlistenFn | null = null
  let unlistenError: UnlistenFn | null = null
  let currentRequestId: string | null = null
  let throttleTimer: ReturnType<typeof setTimeout> | null = null
  let isFirstChunk = true

  let activeResolveCompletion: ((payload: TDone | null) => void) | null = null
  let completionPromise: Promise<TDone | null> = Promise.resolve(null)

  function resolveCompletion(payload: TDone | null) {
    if (activeResolveCompletion) {
      const fn = activeResolveCompletion
      activeResolveCompletion = null
      fn(payload)
    }
  }

  function clearThrottle() {
    if (throttleTimer !== null) {
      clearTimeout(throttleTimer)
      throttleTimer = null
    }
  }

  function cleanupListeners() {
    clearThrottle()
    if (unlistenChunk) { unlistenChunk(); unlistenChunk = null }
    if (unlistenDone) { unlistenDone(); unlistenDone = null }
    if (unlistenError) { unlistenError(); unlistenError = null }
    currentRequestId = null
  }

  function cancel() {
    cleanupListeners()
    loading.value = false
    resolveCompletion(null)
  }

  async function start(
    commandSuffix: string,
    topic: string,
    args: Record<string, unknown>,
    options?: StartOptions
  ): Promise<void> {
    const isAtomic = options?.atomicSwap === true

    // 1. 清掉上一次的 listener 并重置状态
    cleanupListeners()
    resolveCompletion(null)
    if (!isAtomic) {
      items.value = []
    }
    done.value = null
    error.value = null
    loading.value = true
    isFirstChunk = true

    completionPromise = new Promise<TDone | null>((resolve) => {
      activeResolveCompletion = resolve
    })
    const requestCompletion = completionPromise

    const stagedItems: TChunk[] = []

    // 2. 生成 request_id 并订阅事件
    const id = crypto.randomUUID()
    currentRequestId = id
    const chunkEvent = `${topic}:${id}:chunk`
    const doneEvent = `${topic}:${id}:done`
    const errorEvent = `${topic}:${id}:error`

    const [chunkUn, doneUn, errUn] = await Promise.all([
      listen<TChunk[]>(chunkEvent, (ev) => {
        if (currentRequestId !== id) return
        if (isAtomic) {
          // SWR 模式：暂存在缓冲区，不干扰旧数据渲染
          stagedItems.push(...ev.payload)
        } else {
          // shallowRef + 原地 push + triggerRef：O(B) per chunk，避免 concat 整列表 O(N²) 累积
          items.value.push(...ev.payload)
          if (isFirstChunk) {
            isFirstChunk = false
            triggerRef(items)
          } else if (throttleTimer === null) {
            throttleTimer = setTimeout(() => {
              throttleTimer = null
              if (currentRequestId === id) {
                triggerRef(items)
              }
            }, 60)
          }
        }
      }),
      listen<TDone>(doneEvent, (ev) => {
        if (currentRequestId !== id) return
        clearThrottle()
        if (isAtomic) {
          // 原子替换并通知 Vue
          items.value = stagedItems
        }
        triggerRef(items)
        done.value = ev.payload
        loading.value = false
        cleanupListeners()
        resolveCompletion(ev.payload)
      }),
      listen<AppErrorPayload>(errorEvent, (ev) => {
        if (currentRequestId !== id) return
        clearThrottle()
        // 载荷是 AppError 的线格式（`{ code, params }` 或裸字符串），渲染交给
        // wrapAppError——它认这两种形状，缺一种就会把对象直接 String() 成 [object Object]。
        // 结构化载荷的 code 挂在 Error 上留给消费点（按协议状态分支，不是按文案找子串）。
        error.value = wrapAppError(ev.payload)
        loading.value = false
        cleanupListeners()
        resolveCompletion(null)
      }),
    ])

    // 三个 listener 注册期间如果 start/cancel/unmount 抢跑了，立即丢弃
    if (currentRequestId !== id) {
      chunkUn(); doneUn(); errUn()
      resolveCompletion(null)
      return
    }
    unlistenChunk = chunkUn
    unlistenDone = doneUn
    unlistenError = errUn

    // 3. 调用后端，注意必须等 listener 注册完毕后再 invoke
    try {
      // Tauri 命令通常会在后台任务启动后立即返回；但系统繁忙时 invoke
      // 的响应也可能晚于 done/error 事件。终态既然已经到达，就不能继续
      // 卡在 invoke 上，否则刷新按钮会永远保持 loading。
      await Promise.race([
        invokeApp(`${commandSuffix}_stream`, { requestId: id, ...args }),
        requestCompletion.then(() => undefined),
      ])
    } catch (e: unknown) {
      if (currentRequestId !== id) {
        resolveCompletion(null)
        return
      }
      error.value = e instanceof Error ? e : new Error(renderAppError(e))
      loading.value = false
      cleanupListeners()
      resolveCompletion(null)
    }
  }

  function waitForCompletion(timeoutMs = 8000): Promise<TDone | null> {
    if (!loading.value) {
      return Promise.resolve(done.value)
    }
    let timer: ReturnType<typeof setTimeout> | null = null
    const timeoutPromise = new Promise<TDone | null>((resolve) => {
      timer = setTimeout(() => {
        timer = null
        resolve(null)
      }, timeoutMs)
    })
    return Promise.race([
      completionPromise.then((res) => {
        if (timer) {
          clearTimeout(timer)
          timer = null
        }
        return res
      }),
      timeoutPromise,
    ])
  }

  if (getCurrentInstance()) {
    onUnmounted(() => {
      cancel()
    })
  }

  return { items, done, error, loading, start, cancel, waitForCompletion }
}
