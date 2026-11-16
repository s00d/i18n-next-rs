// @vitest-environment happy-dom
// `connect` with the other adapters (DESIGN 8b.20): the instances of `@i18n-micro/react`,
// `/preact` and `/solid` fit its duck interface as they are. In each, a component under
// `I18nProvider` shows the Rust catalog, a page switch and a locale switch re-render it.
import { afterEach, describe, expect, it } from 'vitest'
import { connect, type I18nInstance } from '../../guest-js/index.js'
import { type Catalog, FakePlugin, catalogSource } from './fake.js'

const cat: Catalog = {
  en: { index: { title: 'Home', apples: 'no apples | one apple | {count} apples' }, settings: { title: 'Settings' } },
  de: { index: { title: 'Start' }, settings: { title: 'Einstellungen' } },
}

let fake: FakePlugin
afterEach(() => fake.uninstall())

let live: Catalog
function setup() {
  live = structuredClone(cat)
  fake = new FakePlugin(catalogSource(live, { en: ['en'], de: ['de', 'en'] }, ['index', 'settings']), 'en')
  fake.install()
}

/** The steps every adapter goes through; `render` flushes the framework's updates. */
async function story(i18n: I18nInstance, text: () => string, flush: (f: () => Promise<void>) => Promise<void>) {
  const link = await connect(i18n, { document: null, reportMissing: false })
  await flush(async () => {})
  expect(text()).toBe('Home · 2 apples')
  await flush(() => link.setPage('settings'))
  expect(text()).toBe('Settings · 2 apples')
  await flush(() => link.setLocale('de'))
  expect(text()).toBe('Einstellungen · 2 apples') // `apples` from the chain: de → en
  // the shown page loses its chunks in Rust: the component re-renders from the root
  delete live.de!.settings
  delete live.en!.settings
  fake.bump()
  await flush(async () => {
    await fake.deliver(fake.pending.length - 1) // the newest: the switch's own event is late
    await link.setPage('settings')
  })
  expect(text()).toBe('Start · 2 apples')
  link.close()
}

describe('connect with other adapters', () => {
  it('@i18n-micro/react', async () => {
    setup()
    const React = await import('react')
    const { createRoot } = await import('react-dom/client')
    const { createI18n, I18nProvider, I18nT, useT } = await import('@i18n-micro/react')
    ;(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true
    const i18n = createI18n({ locale: 'en', missingWarn: false })
    const el = document.createElement('div')
    const root = createRoot(el)
    const Title = () => React.createElement('span', null, useT('title'), ' · ', React.createElement(I18nT, { keypath: 'apples', plural: 2, tag: 'i' }))
    await story(
      i18n as unknown as I18nInstance,
      () => el.textContent ?? '',
      async (f) => {
        await React.act(async () => {
          await f()
          root.render(React.createElement(I18nProvider, { i18n }, React.createElement(Title)))
        })
      },
    )
    await React.act(async () => root.unmount())
  })

  it('@i18n-micro/preact', async () => {
    setup()
    const { h, render } = await import('preact')
    const { act } = await import('preact/test-utils')
    const { createI18n, I18nProvider, I18nT, useT } = await import('@i18n-micro/preact')
    const i18n = createI18n({ locale: 'en', missingWarn: false })
    const el = document.createElement('div')
    const Title = () => h('span', null, useT('title'), ' · ', h(I18nT, { keypath: 'apples', plural: 2, tag: 'i' }))
    let mounted = false
    await story(
      i18n as unknown as I18nInstance,
      () => el.textContent ?? '',
      async (f) => {
        await act(async () => {
          await f()
          if (!mounted) render(h(I18nProvider, { i18n }, h(Title, null)), el)
          mounted = true
        })
      },
    )
    render(null, el)
  })

  it('@i18n-micro/solid', async () => {
    setup()
    const { createComponent, createRoot } = await import('solid-js')
    const { render } = await import('solid-js/web')
    const { createI18n, I18nProvider, I18nT, useI18n } = await import('@i18n-micro/solid')
    const i18n = createI18n({ locale: 'en', missingWarn: false })
    const el = document.createElement('div')
    let dispose: (() => void) | undefined
    await story(
      i18n as unknown as I18nInstance,
      () => el.textContent ?? '',
      async (f) => {
        await f()
        dispose ??= render(
          () =>
            createComponent(I18nProvider, {
              i18n,
              get children() {
                const { t } = useI18n()
                const span = document.createElement('span')
                createRoot(() => {
                  // a plain effect: the text follows the instance's signals
                  const update = () => (span.firstChild ? (span.firstChild.textContent = String(t('title'))) : span.append(String(t('title'))))
                  update()
                  i18n.subscribe(update)
                })
                return [span, ' · ', createComponent(I18nT, { keypath: 'apples', plural: 2, tag: 'i' })]
              },
            }),
          el,
        )
      },
    )
    dispose?.()
  })
})
