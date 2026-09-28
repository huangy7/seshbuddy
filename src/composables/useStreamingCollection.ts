import { ref, shallowRef, triggerRef, watch, type Ref, type ShallowRef } from 'vue'
import { useStreamingLoad } from './useStreamingLoad'

export interface UseStreamingCollectionOptions {
  /** 后端命令名（不含 `_stream`），如 `'scan_projects'` */
  command: string
  /** 事件 topic（与后端约定），通常等同于 command */
  topic: string
  /** 显示 loading 指示器的延迟阈值；<= 此值完成的快路径完全沉默；默认 300ms */
  slowThresholdMs?: number
}

export interface StreamingRefreshOptions {
  /**
   * 静默模式（后台定时刷新/文件监听触发）：已有数据时绝不清空列表、
   * 绝不显示 loading pill，新数据到达后原子替换。仅影响展示层，
   * done with 0 chunk 的真清空语义保持不变（数据确实为空时仍会清空）。
   */
  silent?: boolean
  /**
   * 在收到 done 之前不触碰当前集合。适用于前台刷新和文件监听刷新，
   * 避免目录树先显示半份新结果、再随着后续 chunk 反复重排。
   * 首次启动可继续使用渐进式渲染。
   */
  atomicSwap?: boolean
}

export interface UseStreamingCollectionReturn<TItem, TDone> {
  /** 累计数据项；在数据到达前保留上一次的 items，新数据原子替换 */
  items: ShallowRef<TItem[]>
  /** done 事件 payload；refresh 启动时重置为 null */
  done: Ref<TDone | null>
  /** 错误对象 */
  error: Ref<Error | null>
  /** refresh 全程置位的忙碌标志，用于覆盖空状态门控 */
  isRefreshing: Ref<boolean>
  /** 仅在 >slowThresholdMs 仍未收到 chunk 时变 true；快路径永不显示 */
  showLoadingIndicator: Ref<boolean>
  /** 当前已累计 item 数（item 级别，与分组无关） */
  totalItems: Ref<number>
  /** 启动一次 refresh；await 直到 done 或 error 之一到达 */
  refresh: (args?: Record<string, unknown>, opts?: StreamingRefreshOptions) => Promise<void>
  /** 主动取消当前 refresh */
  cancel: () => void
}

/**
 * 流式列表加载通用编排：包装 useStreamingLoad，提供
 * - 慢加载阈值控制的 loading 指示器（避免快路径下 pill 闪烁）
 * - 数据原子替换语义（CLI 切换等场景旧列表保持显示，新数据到达瞬时 swap）
 * - done with 0 chunk 才真清空（区分"加载失败保留旧数据" vs "确实空"）
 * - error 时若一条 chunk 未收到则不动 items（隐式回滚到上次成功状态）
 *
 * **生命周期使用规范**：
 * - 组件 setup 同步阶段调用：依赖 `onUnmounted` 自动 cancel（推荐）
 * - 模块级 / store 风格调用（如 useSessions 单例 store）：getCurrentInstance 返回 null，
 *   `onUnmounted` 注册被跳过；listener 不会自动清理。但 useStreamingLoad.start 内部每次
 *   都会先 cleanupListeners()，所以**不会**累积 listener，单例 store 整个进程生命周期内不需
 *   要清理也安全。代价是失去 unmount 时取消能力——若调用方场景需要主动 cancel 必须自管。
 */
export function useStreamingCollection<TItem, TDone>(
  opts: UseStreamingCollectionOptions
): UseStreamingCollectionReturn<TItem, TDone> {
  const stream = useStreamingLoad<TItem, TDone>()
  const slowThresholdMs = opts.slowThresholdMs ?? 300

  const items = shallowRef<TItem[]>([])
  const done = ref<TDone | null>(null) as Ref<TDone | null>
  const error = ref<Error | null>(null)
  const isRefreshing = ref(false)
  const showLoadingIndicator = ref(false)
  const totalItems = ref(0)

  // 当前在飞 refresh 的「抢占句柄」；同时承担两个职责：
  // 1) cancel() 调用时触发它，解除 refresh 的 await completion 挂起 → 触发 finally 收尾
  // 2) 新 refresh 入口抢占式触发它，让旧 refresh 立刻释放 watcher，避免新旧并存
  let activeAbort: (() => void) | null = null
  // 递增代次计数器：保证较早发起的 refresh 其异步回调或 finally 收尾不会改写最新一次 refresh 的状态
  let refreshGeneration = 0

  function cancel() {
    refreshGeneration += 1
    if (activeAbort) {
      activeAbort()
      activeAbort = null
    }
    stream.cancel()
    isRefreshing.value = false
    showLoadingIndicator.value = false
  }

  async function refresh(
    args: Record<string, unknown> = {},
    refreshOpts: StreamingRefreshOptions = {}
  ): Promise<void> {
    const generation = ++refreshGeneration
    // 抢占：释放任何在飞的 refresh 的 watcher，避免新旧并存
    if (activeAbort) {
      activeAbort()
      activeAbort = null
    }

    const silent = refreshOpts.silent ?? false
    const atomicSwap = refreshOpts.atomicSwap ?? silent
    isRefreshing.value = true
    done.value = null
    error.value = null
    let receivedAny = false

    const slowTimer = setTimeout(() => {
      if (generation !== refreshGeneration) return
      // 保持 SWR（Stale-While-Revalidate）语义：
      // 无论后台扫描耗时多久，均保留已有旧数据正常展示，绝不暴力清空制造"空会话"闪烁。
      // 仅在原本就无数据时显示加载提示。
      if (!receivedAny && !silent && items.value.length === 0) {
        showLoadingIndicator.value = true
      }
    }, slowThresholdMs)

    let resolveCompletion!: () => void
    let completionTimer: ReturnType<typeof setTimeout> | null = null
    const completion = new Promise<void>((r) => {
      resolveCompletion = () => {
        if (completionTimer) {
          clearTimeout(completionTimer)
          completionTimer = null
        }
        r()
      }
      completionTimer = setTimeout(() => {
        completionTimer = null
        r()
      }, 10000)
    })

    // 非原子模式下，标记 stream 刚被 start() 重置（length 短暂为 0）；
    // 下一次非空 chunk 来时整体 swap。原子模式由 useStreamingLoad 在 done
    // 事件中一次性提交，刷新期间 stream.items 始终保持旧集合。
    let pendingReset = false

    // 直接监听 stream 的三个反应式状态；items.length === 0 时早返回避免 start 重置造成的闪烁
    const stop1 = watch(
      stream.items,
      (streamItems) => {
        if (generation !== refreshGeneration) return
        if (atomicSwap) {
          // atomicSwap 的 stream.items 只会在 done 时改变；这里直接提交完整
          // 结果，不让中间 chunk 影响目录树。
          items.value = streamItems.slice()
          totalItems.value = items.value.length
          if (streamItems.length > 0) receivedAny = true
          return
        }
        if (streamItems.length === 0) {
          // useStreamingLoad.start() 抢占时会先把 stream.items 重置为 []。
          // early-return 保留旧 items 避免抖动；用 pendingReset 标记，下一非空 chunk 触发整体 swap。
          // 真正的"显示空"由 done with !receivedAny 分支处理。
          pendingReset = true
          return
        }
        receivedAny = true
        clearTimeout(slowTimer)
        if (!silent) {
          showLoadingIndicator.value = false
        }

        if (pendingReset) {
          // 新一次 refresh 的第一条 chunk：整体 swap，避免新旧 chunk 内容长度恰好对齐时的混入
          items.value = streamItems.slice()
          pendingReset = false
        } else {
          // 正常增量：从 items.value.length 之后的元素 push 进来
          const delta = streamItems.slice(items.value.length)
          if (delta.length > 0) {
            items.value.push(...delta)
            triggerRef(items)
          }
        }
        totalItems.value = items.value.length
      },
      { flush: 'sync' }
    )

    const stop2 = watch(
      stream.done,
      (d) => {
        if (generation !== refreshGeneration) return
        if (!d) return
        done.value = d
        // done 时若一条 chunk 未收到 → 真清空旧 items
        if (!receivedAny) {
          items.value = []
          totalItems.value = 0
        }
        clearTimeout(slowTimer)
        showLoadingIndicator.value = false
        resolveCompletion()
        // 流已终结：此刻释放 watcher 是安全的，不会再有迟到结果
        stopWatchers()
      },
      { flush: 'sync' }
    )

    const stop3 = watch(
      stream.error,
      (e) => {
        if (generation !== refreshGeneration) return
        if (!e) return
        error.value = e
        // 错误时若一条 chunk 未收到 → 不动 items（保留旧数据）
        clearTimeout(slowTimer)
        showLoadingIndicator.value = false
        resolveCompletion()
        stopWatchers()
      },
      { flush: 'sync' }
    )

    // watcher 的释放时机必须与「流是否真的终结」对齐，而不是与「await 是否返回」对齐。
    // 10 秒兜底计时器只是解除 await 挂起（避免调用方永久挂住），此时扫描往往仍在进行 ——
    // 若在 finally 里无条件拆掉 watcher，迟到的 chunk / done 将无人接收，atomicSwap 模式下
    // items 永不提交，用户看到的是永远停在旧快照的侧边栏。
    let watchersStopped = false
    const stopWatchers = () => {
      if (watchersStopped) return
      watchersStopped = true
      stop1()
      stop2()
      stop3()
      // 抢占句柄只应与 watcher 同生共死：若不在这里一并清掉，超时保活的那批 watcher
      // 会永远无人释放（后续 refresh/cancel 都以为没有在飞任务）—— 而本组件在
      // useSessions 里是模块级单例，进程内不会卸载，泄漏是永久的
      if (activeAbort === abort) {
        activeAbort = null
      }
    }

    // 抢占/取消专用：立刻解除挂起并释放 watcher（旧代次的 watcher 已因代次校验失效，释放即可）
    const abort = () => {
      resolveCompletion()
      stopWatchers()
    }
    activeAbort = abort

    try {
      await stream.start(opts.command, opts.topic, args, { atomicSwap })
      await completion
    } finally {
      if (completionTimer) {
        clearTimeout(completionTimer)
        completionTimer = null
      }
      clearTimeout(slowTimer)
      if (generation === refreshGeneration) {
        showLoadingIndicator.value = false
        isRefreshing.value = false
      }
      // 流已终结、或本次已被抢占 → 释放 watcher（并随之清空抢占句柄）；
      // 兜底超时但扫描仍在跑 → 保留 watcher 让迟到结果照常落地，句柄也一并保留，
      // 这样下一次 refresh/cancel 仍能通过 activeAbort 释放它们。
      if (generation !== refreshGeneration || stream.done.value || stream.error.value) {
        stopWatchers()
      }
    }
  }

  return {
    items,
    done,
    error,
    isRefreshing,
    showLoadingIndicator,
    totalItems,
    refresh,
    cancel,
  }
}
