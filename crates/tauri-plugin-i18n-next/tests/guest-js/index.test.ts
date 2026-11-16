import { afterEach, beforeAll, describe, expect, it } from 'vitest'
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks'
import { meta } from '../../guest-js/index.js'

// mockIPC patches window.__TAURI_INTERNALS__; the node environment has no window.
beforeAll(() => {
  ;(globalThis as { window?: unknown }).window ??= globalThis
})
afterEach(() => clearMocks())

describe('meta: the front end reads what it needs to start', () => {
  it('sends plugin:i18n-next|meta with the names, parses the raw JSON answer; a backend error reaches the caller', async () => {
    const calls: Array<[string, unknown]> = []
    let fail = false
    const answer = {
      version: 3,
      locale: 'de-at',
      chains: { 'de-at': ['de-at', 'de', 'en'] },
      tags: { 'de/index': '0123456789abcdef' },
      bundle: 'fedcba9876543210',
    }
    mockIPC((cmd, args) => {
      calls.push([cmd, args])
      if (fail) throw new Error('UnknownLocale')
      // the plugin answers raw bytes (`tauri::ipc::Response`)
      return new TextEncoder().encode(JSON.stringify(answer)).buffer
    })

    // step 1: the answer is parsed, one IPC call with the given names
    await expect(meta({ locale: 'de-at', page: 'settings' })).resolves.toMatchObject(answer)
    expect(calls).toEqual([['plugin:i18n-next|meta', { locale: 'de-at', page: 'settings' }]])

    // step 2: backend fails → rejection, not a silent value
    fail = true
    await expect(meta()).rejects.toThrow('UnknownLocale')
    expect(calls).toHaveLength(2)
  })
})
