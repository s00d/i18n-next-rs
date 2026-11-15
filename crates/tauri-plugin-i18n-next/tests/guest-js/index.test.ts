import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { ping } from '../../guest-js/index.js'

// mockIPC patches window.__TAURI_INTERNALS__; the node environment has no window.
beforeAll(() => {
  ;(globalThis as { window?: unknown }).window ??= globalThis
})
afterEach(() => clearMocks())

describe('ping: the app checks that the plugin is registered', () => {
  it('sends plugin:i18n-next|ping once and returns the reply; a backend error reaches the caller', async () => {
    const calls: string[] = []
    let fail = false
    mockIPC((cmd) => {
      calls.push(cmd)
      if (fail) throw new Error('plugin not registered')
      return 'pong'
    })

    // step 1: plugin answers → reply as is, one IPC call
    await expect(ping()).resolves.toBe('pong')
    expect(calls).toEqual(['plugin:i18n-next|ping'])

    // step 2: backend fails → rejection, not a silent value
    fail = true
    await expect(ping()).rejects.toThrow('plugin not registered')
    expect(calls).toHaveLength(2)
  })
})
