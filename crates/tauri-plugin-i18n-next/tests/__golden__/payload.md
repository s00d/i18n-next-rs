# Payload schema (characterization golden)

load {}
→ {"version":0,"locale":"en","defaultLocale":"en","fallbackLocale":"en","locales":[{"code":"en","iso":"en-US","displayName":"English"},{"code":"ar","iso":"ar-EG","dir":"rtl"}],"numberFormats":{"ar":{"n":{"maximumFractionDigits":1}},"en":{"money":{"currency":"EUR","style":"currency"}}},"datetimeFormats":{"en":{"short":{"dateStyle":"short"}}},"chunks":[{"locale":"en","page":"index","data":{"a.b":"own dotted key","cart":"=0 empty | {count} item | {count} items","esc":"quote \" slash \\ tab \t nl \n u2028  ","n":1.5,"nav":{"home":"Home","list":["a",{"b":"B"}]},"t":true,"x":null}}]}

load {page: settings}
→ {"version":0,"locale":"en","defaultLocale":"en","fallbackLocale":"en","locales":[{"code":"en","iso":"en-US","displayName":"English"},{"code":"ar","iso":"ar-EG","dir":"rtl"}],"numberFormats":{"ar":{"n":{"maximumFractionDigits":1}},"en":{"money":{"currency":"EUR","style":"currency"}}},"datetimeFormats":{"en":{"short":{"dateStyle":"short"}}},"chunks":[{"locale":"en","page":"index","data":{"a.b":"own dotted key","cart":"=0 empty | {count} item | {count} items","esc":"quote \" slash \\ tab \t nl \n u2028  ","n":1.5,"nav":{"home":"Home","list":["a",{"b":"B"}]},"t":true,"x":null}},{"locale":"en","page":"settings","data":{"title":"Settings"}}]}

load_page {page: settings}
→ {"version":0,"locale":"en","chunks":[{"locale":"en","page":"settings","data":{"title":"Settings"}}]}

load_page {page: nope}
→ {"kind":"UnknownPage","message":"unknown page: nope"}

GET /v0/en/index.json
→ 200 [cache-control: max-age=31536000, immutable; content-type: application/json; etag: "0"; vary: Origin] {"a.b":"own dotted key","cart":"=0 empty | {count} item | {count} items","esc":"quote \" slash \\ tab \t nl \n u2028  ","n":1.5,"nav":{"home":"Home","list":["a",{"b":"B"}]},"t":true,"x":null}

GET /v0/ar/settings.json
→ 200 [cache-control: max-age=31536000, immutable; content-type: application/json; etag: "0"; vary: Origin] {"title":"الإعدادات"}

GET /v0/bundle?locale=ar&page=settings
→ 200 [cache-control: max-age=31536000, immutable; content-type: application/json; etag: "0"; vary: Origin] {"version":0,"locale":"ar","chunks":[{"locale":"ar","page":"index","data":{"nav":{"home":"الرئيسية"}}},{"locale":"ar","page":"settings","data":{"title":"الإعدادات"}},{"locale":"en","page":"index","data":{"a.b":"own dotted key","cart":"=0 empty | {count} item | {count} items","esc":"quote \" slash \\ tab \t nl \n u2028  ","n":1.5,"nav":{"home":"Home","list":["a",{"b":"B"}]},"t":true,"x":null}},{"locale":"en","page":"settings","data":{"title":"Settings"}}]}

GET /v0/xx/index.json
→ 404 [cache-control: no-store; content-type: application/json; vary: Origin] {"kind":"UnknownLocale","message":"unknown locale: xx"}

GET /v1/en/index.json
→ 410 [cache-control: no-store; content-type: application/json; vary: Origin; x-i18n-version: 0] {"kind":"Gone","message":"the catalog has a newer version"}

event after set_locale ar
→ [{"locale":"ar","version":1}]

available_locales
→ [{"code":"en","displayName":"English","iso":"en-US"},{"code":"ar","dir":"rtl","iso":"ar-EG"}]

pages
→ ["settings"]

