# Payload schema (characterization golden)

load {}
→ {"version":0,"locale":"en","defaultLocale":"en","fallbackLocale":"en","locales":[{"code":"en","iso":"en-US","displayName":"English"},{"code":"ar","iso":"ar-EG","dir":"rtl"}],"chains":{"en":["en"],"ar":["ar","en"]},"numberFormats":{"ar":{"n":{"maximumFractionDigits":1}},"en":{"money":{"currency":"EUR","style":"currency"}}},"datetimeFormats":{"en":{"short":{"dateStyle":"short"}}},"chunks":[{"locale":"en","page":"index","data":{"a.b":"own dotted key","brand":"Acme","cart":"=0 empty | {count} item | {count} items","esc":"quote \" slash \\ tab \t nl \n u2028  ","hello":"Hi Acme","inf":null,"n":1.5,"nav":{"home":"Home","list":["a",{"b":"B"}]},"t":true,"x":null}}]}

load {page: settings}
→ {"version":0,"locale":"en","defaultLocale":"en","fallbackLocale":"en","locales":[{"code":"en","iso":"en-US","displayName":"English"},{"code":"ar","iso":"ar-EG","dir":"rtl"}],"chains":{"en":["en"],"ar":["ar","en"]},"numberFormats":{"ar":{"n":{"maximumFractionDigits":1}},"en":{"money":{"currency":"EUR","style":"currency"}}},"datetimeFormats":{"en":{"short":{"dateStyle":"short"}}},"chunks":[{"locale":"en","page":"index","data":{"a.b":"own dotted key","brand":"Acme","cart":"=0 empty | {count} item | {count} items","esc":"quote \" slash \\ tab \t nl \n u2028  ","hello":"Hi Acme","inf":null,"n":1.5,"nav":{"home":"Home","list":["a",{"b":"B"}]},"t":true,"x":null}},{"locale":"en","page":"settings","data":{"title":"Settings"}}]}

load_page {page: settings}
→ {"version":0,"locale":"en","chunks":[{"locale":"en","page":"settings","data":{"title":"Settings"}}]}

meta {locale: ar, page: settings}
→ {"version":0,"locale":"ar","defaultLocale":"en","fallbackLocale":"en","locales":[{"code":"en","iso":"en-US","displayName":"English"},{"code":"ar","iso":"ar-EG","dir":"rtl"}],"chains":{"en":["en"],"ar":["ar","en"]},"numberFormats":{"ar":{"n":{"maximumFractionDigits":1}},"en":{"money":{"currency":"EUR","style":"currency"}}},"datetimeFormats":{"en":{"short":{"dateStyle":"short"}}},"tags":{"ar/index":"504c298e90f12541","ar/settings":"add18dafe3edc5b5","en/index":"02a427886673e95d","en/settings":"61aa6433a746b73a"},"bundle":"81b4e1197b69e1a3"}

load_page {page: nope}
→ {"kind":"UnknownPage","message":"unknown page: nope"}

GET /02a427886673e95d/en/index.json
→ 200 [access-control-expose-headers: etag, x-i18n-tag, x-i18n-version; cache-control: max-age=31536000, immutable; content-type: application/json; etag: "02a427886673e95d"; vary: Origin] {"data":{"a.b":"own dotted key","brand":"Acme","cart":"=0 empty | {count} item | {count} items","esc":"quote \" slash \\ tab \t nl \n u2028  ","hello":"Hi Acme","inf":null,"n":1.5,"nav":{"home":"Home","list":["a",{"b":"B"}]},"t":true,"x":null}}

GET /504c298e90f12541/ar/index.json
→ 200 [access-control-expose-headers: etag, x-i18n-tag, x-i18n-version; cache-control: max-age=31536000, immutable; content-type: application/json; etag: "504c298e90f12541"; vary: Origin] {"data":{"brand":"أكمي","nav":{"home":"الرئيسية"}},"linked":{"hello":"Hi أكمي"}}

GET /add18dafe3edc5b5/ar/settings.json
→ 200 [access-control-expose-headers: etag, x-i18n-tag, x-i18n-version; cache-control: max-age=31536000, immutable; content-type: application/json; etag: "add18dafe3edc5b5"; vary: Origin] {"data":{"title":"الإعدادات"},"linked":{"hello":"Hi أكمي"}}

GET /81b4e1197b69e1a3/bundle?locale=ar&page=settings
→ 200 [access-control-expose-headers: etag, x-i18n-tag, x-i18n-version; cache-control: max-age=31536000, immutable; content-type: application/json; etag: "81b4e1197b69e1a3"; vary: Origin] {"locale":"ar","chunks":[{"locale":"ar","page":"index","data":{"brand":"أكمي","nav":{"home":"الرئيسية"}},"linked":{"hello":"Hi أكمي"}},{"locale":"ar","page":"settings","data":{"title":"الإعدادات"},"linked":{"hello":"Hi أكمي"}},{"locale":"en","page":"index","data":{"a.b":"own dotted key","brand":"Acme","cart":"=0 empty | {count} item | {count} items","esc":"quote \" slash \\ tab \t nl \n u2028  ","hello":"Hi Acme","inf":null,"n":1.5,"nav":{"home":"Home","list":["a",{"b":"B"}]},"t":true,"x":null}},{"locale":"en","page":"settings","data":{"title":"Settings"}}]}

GET /02a427886673e95d/xx/index.json
→ 404 [access-control-expose-headers: etag, x-i18n-tag, x-i18n-version; cache-control: no-store; content-type: application/json; vary: Origin] {"kind":"UnknownLocale","message":"unknown locale: xx"}

GET /0000000000000000/en/index.json
→ 410 [access-control-expose-headers: etag, x-i18n-tag, x-i18n-version; cache-control: no-store; content-type: application/json; vary: Origin; x-i18n-tag: 02a427886673e95d; x-i18n-version: 0] {"kind":"Gone","message":"the resource changed: read it under its current tag","tag":"02a427886673e95d","version":0}

OPTIONS (x-trace)
→ 204 [access-control-allow-headers: x-trace; access-control-allow-methods: GET, HEAD, OPTIONS; access-control-expose-headers: etag, x-i18n-tag, x-i18n-version; access-control-max-age: 86400; cache-control: no-store; content-type: application/json; vary: Origin] 

event after set_locale ar
→ [{"locale":"ar","version":1}]

available_locales
→ [{"code":"en","displayName":"English","iso":"en-US"},{"code":"ar","dir":"rtl","iso":"ar-EG"}]

pages
→ ["settings"]

