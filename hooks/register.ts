import type { EngineInterface, Register } from 'claude-code'

function stripAnsi(s: string): string {
  return s.replace(/\x1b\[[0-9;?]*[A-Za-z]/g, '').trim()
}

function splitArgs(raw: string): string[] {
  return raw.trim().split(/\s+/).filter(Boolean)
}

async function resolveIpcDir($: EngineInterface): Promise<string> {
  const custom = await $.env.get('CCIDE_IPC_DIR')
  if (custom) return custom
  const winTmp = (await $.env.get('TEMP')) || (await $.env.get('TMP'))
  const userProfile = await $.env.get('USERPROFILE')
  if (winTmp && userProfile) return `${winTmp.replace(/[\\/]+$/, '')}/.cc-ide`
  return '/tmp/.cc-ide'
}

async function resolveHomeDir($: EngineInterface): Promise<string> {
  return (await $.env.get('HOME')) || (await $.env.get('USERPROFILE')) || ''
}

async function runCore(
  $: EngineInterface,
  resolverPath: string,
  subArgs: readonly string[],
): Promise<{ code: number; out: string }> {
  const res = await $.process.run(['node', resolverPath, ...subArgs], { timeoutMs: 15000 })
  const out = [stripAnsi(res.stdout), stripAnsi(res.stderr)].filter(Boolean).join('\n')
  return { code: res.exitCode, out }
}

const lastModels = new Map<string, string>()

async function syncSessionModel($: EngineInterface, overrideModel?: string): Promise<void> {
  try {
    const [sid, curModel, home, ipcDir] = await Promise.all([
      $.session.id(),
      overrideModel ? Promise.resolve(overrideModel) : $.session.model(),
      resolveHomeDir($),
      resolveIpcDir($),
    ])
    const m = (curModel || '').trim()
    if (!sid || !m || m.startsWith('<') || lastModels.get(sid) === m) return
    lastModels.set(sid, m)
    const writes: Promise<unknown>[] = [$.fs.write(`${ipcDir}/mod_model_${sid}.txt`, `${m}\n`)]
    if (home) writes.push($.fs.write(`${home}/.cache/cc-ide/status/model_${sid}.cache`, `${m}\n`))
    await Promise.all(writes)
    const resolver = `${$.plugin.root}/scripts/ensure-ccide-core.mjs`
    const sidPid = (await $.fs.read(`${ipcDir}/sb_${sid}.pid`).catch(() => '')).trim()
    const rawPids = sidPid
      ? [sidPid]
      : await $.fs
          .list(ipcDir)
          .catch(() => [])
          .then(ents =>
            Promise.all(
              ents
                .map(ent => ent.name)
                .filter(n => n.startsWith('sb_yazi_') && n.endsWith('.pid'))
                .map(n => $.fs.read(`${ipcDir}/${n}`).catch(() => '')),
            ),
          )
    const pids = rawPids.map(t => t.trim()).filter(p => /^\d+$/.test(p) && Number(p) > 1)
    await Promise.all(
      pids.map(pid =>
        $.process.run(['node', resolver, 'wake-pid', pid], { timeoutMs: 2000 }).catch(() => undefined),
      ),
    )
  } catch {
    // Non-fatal telemetry sync
  }
}

const perfBuffers = new Map<string, string[]>()
const perfQueues = new Map<string, Promise<void>>()

async function appendPerfSample(
  $: EngineInterface,
  sample: { m: string; o: number; d: number; t: number; f: number },
): Promise<void> {
  try {
    const [sid, ipcDir] = await Promise.all([$.session.id(), resolveIpcDir($)])
    if (!sid) return
    const prev = perfQueues.get(sid) ?? Promise.resolve()
    const next = prev
      .then(async () => {
        const pfPath = `${ipcDir}/pf_sid_${sid}.jsonl`
        let lines = perfBuffers.get(sid)
        if (!lines) {
          const existing = await $.fs.read(pfPath).catch(() => '')
          lines = existing.split('\n').map(s => s.trim()).filter(Boolean)
          perfBuffers.set(sid, lines)
        }
        lines.push(JSON.stringify(sample))
        await $.fs.write(pfPath, `${lines.join('\n')}\n`)
      })
      .catch(() => undefined)
    perfQueues.set(sid, next)
    await next
  } catch {
    // Non-fatal perf stream write
  }
}

export const register: Register = on => {
  on('session.start', async ($, e, next) => {
    const cmds = [
      {
        name: 'palette',
        description:
          'Switch or inspect cc-ide color & syntax palette (default, catppuccin-mocha, tokyo-night, nord, gruvbox-dark, dracula)',
        argumentHint: '[preset|list|sync|reset]',
      },
      {
        name: 'rebalance-panes',
        description: 'Rebalance open cc-ide panes to 30% | 54% | 16%',
      },
      {
        name: 'clear-agents',
        description: 'Dismiss completed/stopped subagents from Right Sidebar',
      },
      {
        name: 'orch',
        description:
          'Configure cc-ide subagent & Workflow model/effort orchestrator',
        argumentHint: '[status|balanced|fast|deep-audit|hetero-rr|set ...]',
      },
      {
        name: 'ccide-doctor',
        description:
          'Instant cc-ide workspace diagnostics & controller (doctor, theme, rebalance, clear, orch)',
        argumentHint: '[doctor|theme <name>|rebalance|clear|orch ...]',
      },
    ] as const
    for (const cmd of cmds) {
      try {
        await $.command.register({ ...cmd, immediate: true })
      } catch {
        // Ignore if another mod or skill already claimed the command name
      }
    }
    await syncSessionModel($)
    return next(e)
  })

  on('classic.PostModelSwitch', async ($, e, next) => {
    await syncSessionModel($, e.to_model)
    return next(e)
  }).catch(($, e, next) => (next.called ? next(e) : {}))

  // Official streaming hook replacing Binary Patch #7 (`export_api_perf_metrics`):
  // records monotonic `ttftMs`, `firstContentMs`, and `durMs` for Main + Subagent steps.
  on('turn.step', async function* ($, e, next) {
    const t0 = await $.clock.now()
    let ttftMs = 0
    let firstContentMs = 0
    const stream = next(e)
    for await (const chunk of stream) {
      if (
        (chunk.kind === 'thinking' ||
          chunk.kind === 'text' ||
          chunk.kind === 'tool' ||
          chunk.kind === 'input') &&
        ttftMs === 0
      ) {
        ttftMs = Math.max(1, (await $.clock.now()) - t0)
      }
      if (
        (chunk.kind === 'text' || chunk.kind === 'tool') &&
        firstContentMs === 0
      ) {
        firstContentMs = Math.max(1, (await $.clock.now()) - t0)
      }
      yield chunk
    }
    const res = await stream.result
    const durMs = Math.max(1, (await $.clock.now()) - t0)
    const outTok = res.usage?.output_tokens ?? 0
    if (outTok > 0 || ttftMs > 0) {
      await appendPerfSample($, {
        m: res.usage?.model || e.model,
        o: outTok,
        d: durMs,
        t: ttftMs,
        f: firstContentMs || ttftMs,
      })
    }
    return res
  })

  on('command.run', { command: 'palette' }, async ($, e) => {
    const resolver = `${$.plugin.root}/scripts/ensure-ccide-core.mjs`
    const sub = e.args.trim() || 'list'
    const { code, out } = await runCore($, resolver, [
      'workspace',
      'theme',
      ...splitArgs(sub),
    ])
    if (code === 0 && sub !== 'list' && sub !== 'path') {
      $.ui.toast(`cc-ide palette: ${sub}`)
    }
    return { text: out || `Palette command '${sub}' completed.`, exitCode: code }
  })

  on('command.run', { command: 'rebalance-panes' }, async $ => {
    const resolver = `${$.plugin.root}/scripts/ensure-ccide-core.mjs`
    const { code, out } = await runCore($, resolver, ['workspace', 'rebalance'])
    $.ui.toast('Rebalanced cc-ide columns (30% | 54% | 16%)')
    return {
      text: out || 'Rebalanced open cc-ide workspace panes (30% | 54% | 16%).',
      exitCode: code,
    }
  })

  on('command.run', { command: 'clear-agents' }, async $ => {
    const resolver = `${$.plugin.root}/scripts/ensure-ccide-core.mjs`
    const { code, out } = await runCore($, resolver, [
      'workspace',
      'clear-subagents',
    ])
    if (out) $.ui.toast(out)
    return { text: out || 'Cleared completed subagents.', exitCode: code }
  })

  on('command.run', { command: 'orch' }, async ($, e) => {
    const resolver = `${$.plugin.root}/scripts/ensure-ccide-core.mjs`
    const parts = splitArgs(e.args)
    const presets = ['balanced', 'fast', 'deep-audit', 'hetero-rr']
    const orchArgs =
      parts.length === 0
        ? ['orchestrator', 'status']
        : parts.length === 1 && presets.includes(parts[0])
          ? ['orchestrator', 'preset', parts[0]]
          : ['status', 'preset', 'set', 'sync'].includes(parts[0])
            ? ['orchestrator', ...parts]
            : ['orchestrator', 'set', ...parts]
    const { code, out } = await runCore($, resolver, orchArgs)
    if (code === 0 && orchArgs[1] !== 'status') {
      $.ui.toast(`cc-ide orch: ${orchArgs.slice(1).join(' ')}`)
    }
    return { text: out || 'Orchestrator updated.', exitCode: code }
  })

  on('command.run', { command: 'ccide-doctor' }, async ($, e) => {
    const resolver = `${$.plugin.root}/scripts/ensure-ccide-core.mjs`
    const parts = splitArgs(e.args)
    const head = (parts[0] || 'doctor').toLowerCase()
    if (head === 'theme' || head === 'palette') {
      const target = parts[1] || 'list'
      const { code, out } = await runCore($, resolver, [
        'workspace',
        'theme',
        target,
      ])
      if (code === 0 && target !== 'list') $.ui.toast(`cc-ide palette: ${target}`)
      return { text: out || `Palette: ${target}`, exitCode: code }
    }
    if (head === 'rebalance') {
      const { code, out } = await runCore($, resolver, ['workspace', 'rebalance'])
      $.ui.toast('Rebalanced cc-ide columns')
      return { text: out || 'Rebalanced workspace panes.', exitCode: code }
    }
    if (head === 'clear' || head === 'clear-subagents') {
      const { code, out } = await runCore($, resolver, [
        'workspace',
        'clear-subagents',
      ])
      return { text: out || 'Cleared completed subagents.', exitCode: code }
    }
    if (head === 'orch' || head === 'orchestrator') {
      const rest = parts.slice(1)
      const { code, out } = await runCore($, resolver, [
        'orchestrator',
        ...(rest.length ? rest : ['status']),
      ])
      return { text: out || 'Orchestrator updated.', exitCode: code }
    }
    const { code, out } = await runCore($, resolver, [
      'doctor',
      $.plugin.root,
      ...parts.slice(head === 'doctor' ? 1 : 0),
    ])
    return { text: out || 'cc-ide doctor finished.', exitCode: code }
  })
}
