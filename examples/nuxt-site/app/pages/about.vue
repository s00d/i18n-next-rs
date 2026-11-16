<!-- The texts of playground-rust `page_texts("about")`, key for key (data-k). -->
<template>
  <main>
    <h1 data-k="title">{{ $t('title') }}</h1>
    <p data-k="body">{{ $t('body', { brand: 'i18n-next' }) }}</p>
    <p data-k="source">{{ $t('source') }}</p>
  </main>
</template>
